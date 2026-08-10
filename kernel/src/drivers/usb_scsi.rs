use crate::drivers::usb_bulk_only::{
    BotCommandStatus, BotDataPhase, BotError, BulkOnlyTransport, UsbBulkPipe,
};
use crate::storage::block::{BLOCK_SIZE, BlockDevice, BlockError};

const INQUIRY_OPCODE: u8 = 0x12;
const TEST_UNIT_READY_OPCODE: u8 = 0x00;
const READ_CAPACITY_10_OPCODE: u8 = 0x25;
const READ_10_OPCODE: u8 = 0x28;
const WRITE_10_OPCODE: u8 = 0x2a;
const REQUEST_SENSE_OPCODE: u8 = 0x03;
const SYNCHRONIZE_CACHE_10_OPCODE: u8 = 0x35;
const INQUIRY_LENGTH: usize = 36;
const READ_CAPACITY_10_LENGTH: usize = 8;
const REQUEST_SENSE_LENGTH: usize = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScsiSenseData {
    pub response_code: u8,
    pub valid: bool,
    pub sense_key: u8,
    pub additional_sense_code: u8,
    pub additional_sense_qualifier: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScsiInquiryData {
    pub peripheral_qualifier: u8,
    pub peripheral_device_type: u8,
    pub removable: bool,
    pub version: u8,
    pub response_data_format: u8,
    pub vendor: [u8; 8],
    pub product: [u8; 16],
    pub revision: [u8; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScsiCapacity {
    pub last_lba: u32,
    pub block_count: u64,
    pub block_size: u32,
}

impl ScsiInquiryData {
    pub fn vendor_str(&self) -> &str {
        trimmed_ascii(&self.vendor)
    }

    pub fn product_str(&self) -> &str {
        trimmed_ascii(&self.product)
    }

    pub fn revision_str(&self) -> &str {
        trimmed_ascii(&self.revision)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScsiError<E> {
    Transport(BotError<E>),
    CheckCondition { residue: u32 },
    InvalidCapacity,
    CapacityExceedsRead10,
    CapacityUnknown,
    InvalidTransferLength,
    AddressOutOfRange,
    UnsupportedDevice,
    UnsupportedBlockSize,
}

pub struct UsbScsiDevice<P> {
    transport: BulkOnlyTransport<P>,
    lun: u8,
    capacity: Option<ScsiCapacity>,
}

/// Read-only 512-byte block view over a probed USB SCSI transparent device.
pub struct UsbMassStorageBlockDevice<P> {
    device: UsbScsiDevice<P>,
    blocks: u64,
}

/// Explicitly writable USB block view. Every successful sector write is
/// followed by SYNCHRONIZE CACHE(10) before returning to the caller.
pub struct UsbMassStorageWritableBlockDevice<P> {
    device: UsbScsiDevice<P>,
    blocks: u64,
}

impl<P: UsbBulkPipe> UsbMassStorageWritableBlockDevice<P> {
    pub fn probe(mut device: UsbScsiDevice<P>) -> Result<Self, ScsiError<P::Error>> {
        let inquiry = device.inquiry()?;
        if inquiry.peripheral_qualifier != 0 || inquiry.peripheral_device_type != 0 {
            return Err(ScsiError::UnsupportedDevice);
        }
        device.test_unit_ready()?;
        let capacity = device.read_capacity_10()?;
        if capacity.block_size as usize != BLOCK_SIZE {
            return Err(ScsiError::UnsupportedBlockSize);
        }
        Ok(Self {
            device,
            blocks: capacity.block_count,
        })
    }

    pub fn into_scsi_device(self) -> UsbScsiDevice<P> {
        self.device
    }
}

impl<P: UsbBulkPipe> BlockDevice for UsbMassStorageWritableBlockDevice<P> {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(
        &mut self,
        block_index: u64,
        buffer: &mut [u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        if block_index >= self.blocks || block_index > u64::from(u32::MAX) {
            return Err(BlockError::OutOfRange);
        }
        self.device
            .read_10(block_index as u32, 1, buffer)
            .map_err(|_| BlockError::DeviceFault)
    }

    fn write_block(
        &mut self,
        block_index: u64,
        buffer: &[u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        if block_index >= self.blocks || block_index > u64::from(u32::MAX) {
            return Err(BlockError::OutOfRange);
        }
        self.device
            .write_10(block_index as u32, 1, buffer)
            .and_then(|_| self.device.synchronize_cache_10())
            .map_err(|_| BlockError::DeviceFault)
    }
}

impl<P: UsbBulkPipe> UsbMassStorageBlockDevice<P> {
    pub fn probe(mut device: UsbScsiDevice<P>) -> Result<Self, ScsiError<P::Error>> {
        let inquiry = device.inquiry()?;
        if inquiry.peripheral_qualifier != 0 || inquiry.peripheral_device_type != 0 {
            return Err(ScsiError::UnsupportedDevice);
        }
        device.test_unit_ready()?;
        let capacity = device.read_capacity_10()?;
        if capacity.block_size as usize != BLOCK_SIZE {
            return Err(ScsiError::UnsupportedBlockSize);
        }
        Ok(Self {
            device,
            blocks: capacity.block_count,
        })
    }

    pub fn into_scsi_device(self) -> UsbScsiDevice<P> {
        self.device
    }
}

impl<P: UsbBulkPipe> BlockDevice for UsbMassStorageBlockDevice<P> {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(
        &mut self,
        block_index: u64,
        buffer: &mut [u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        if block_index >= self.blocks || block_index > u64::from(u32::MAX) {
            return Err(BlockError::OutOfRange);
        }
        self.device
            .read_10(block_index as u32, 1, buffer)
            .map_err(|_| BlockError::DeviceFault)
    }

    fn write_block(
        &mut self,
        _block_index: u64,
        _buffer: &[u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        Err(BlockError::ReadOnly)
    }
}

impl<P: UsbBulkPipe> UsbScsiDevice<P> {
    pub const fn new(transport: BulkOnlyTransport<P>, lun: u8) -> Self {
        Self {
            transport,
            lun,
            capacity: None,
        }
    }

    pub fn inquiry(&mut self) -> Result<ScsiInquiryData, ScsiError<P::Error>> {
        let command = [INQUIRY_OPCODE, 0, 0, 0, INQUIRY_LENGTH as u8, 0];
        let mut response = [0u8; INQUIRY_LENGTH];
        let status = self
            .transport
            .command(self.lun, &command, BotDataPhase::In(&mut response))
            .map_err(ScsiError::Transport)?;
        require_passed(status)?;

        let mut vendor = [0u8; 8];
        vendor.copy_from_slice(&response[8..16]);
        let mut product = [0u8; 16];
        product.copy_from_slice(&response[16..32]);
        let mut revision = [0u8; 4];
        revision.copy_from_slice(&response[32..36]);

        Ok(ScsiInquiryData {
            peripheral_qualifier: response[0] >> 5,
            peripheral_device_type: response[0] & 0x1f,
            removable: response[1] & 0x80 != 0,
            version: response[2],
            response_data_format: response[3] & 0x0f,
            vendor,
            product,
            revision,
        })
    }

    pub fn test_unit_ready(&mut self) -> Result<(), ScsiError<P::Error>> {
        let command = [TEST_UNIT_READY_OPCODE, 0, 0, 0, 0, 0];
        let status = self
            .transport
            .command(self.lun, &command, BotDataPhase::None)
            .map_err(ScsiError::Transport)?;
        require_passed(status)
    }

    pub fn request_sense(&mut self) -> Result<ScsiSenseData, ScsiError<P::Error>> {
        let command = [REQUEST_SENSE_OPCODE, 0, 0, 0, REQUEST_SENSE_LENGTH as u8, 0];
        let mut response = [0u8; REQUEST_SENSE_LENGTH];
        let status = self
            .transport
            .command(self.lun, &command, BotDataPhase::In(&mut response))
            .map_err(ScsiError::Transport)?;
        require_passed(status)?;
        Ok(ScsiSenseData {
            response_code: response[0] & 0x7f,
            valid: response[0] & 0x80 != 0,
            sense_key: response[2] & 0x0f,
            additional_sense_code: response[12],
            additional_sense_qualifier: response[13],
        })
    }

    pub fn read_capacity_10(&mut self) -> Result<ScsiCapacity, ScsiError<P::Error>> {
        let command = [READ_CAPACITY_10_OPCODE, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut response = [0u8; READ_CAPACITY_10_LENGTH];
        let status = self
            .transport
            .command(self.lun, &command, BotDataPhase::In(&mut response))
            .map_err(ScsiError::Transport)?;
        require_passed(status)?;

        let last_lba = read_be_u32(&response, 0);
        if last_lba == u32::MAX {
            return Err(ScsiError::CapacityExceedsRead10);
        }
        let block_size = read_be_u32(&response, 4);
        if block_size == 0 {
            return Err(ScsiError::InvalidCapacity);
        }
        let capacity = ScsiCapacity {
            last_lba,
            block_count: u64::from(last_lba) + 1,
            block_size,
        };
        self.capacity = Some(capacity);
        Ok(capacity)
    }

    pub fn read_10(
        &mut self,
        start_lba: u32,
        block_count: u16,
        out: &mut [u8],
    ) -> Result<(), ScsiError<P::Error>> {
        let capacity = self.capacity.ok_or(ScsiError::CapacityUnknown)?;
        if block_count == 0 {
            return Err(ScsiError::InvalidTransferLength);
        }
        let expected_length = usize::try_from(capacity.block_size)
            .ok()
            .and_then(|size| size.checked_mul(usize::from(block_count)))
            .ok_or(ScsiError::InvalidTransferLength)?;
        if out.len() != expected_length {
            return Err(ScsiError::InvalidTransferLength);
        }
        let last_lba = start_lba
            .checked_add(u32::from(block_count) - 1)
            .ok_or(ScsiError::AddressOutOfRange)?;
        if last_lba > capacity.last_lba {
            return Err(ScsiError::AddressOutOfRange);
        }

        let lba = start_lba.to_be_bytes();
        let blocks = block_count.to_be_bytes();
        let command = [
            READ_10_OPCODE,
            0,
            lba[0],
            lba[1],
            lba[2],
            lba[3],
            0,
            blocks[0],
            blocks[1],
            0,
        ];
        let status = self
            .transport
            .command(self.lun, &command, BotDataPhase::In(out))
            .map_err(ScsiError::Transport)?;
        require_passed(status)
    }

    pub fn write_10(
        &mut self,
        start_lba: u32,
        block_count: u16,
        data: &[u8],
    ) -> Result<(), ScsiError<P::Error>> {
        self.validate_transfer(start_lba, block_count, data.len())?;
        let lba = start_lba.to_be_bytes();
        let blocks = block_count.to_be_bytes();
        let command = [
            WRITE_10_OPCODE,
            0,
            lba[0],
            lba[1],
            lba[2],
            lba[3],
            0,
            blocks[0],
            blocks[1],
            0,
        ];
        let status = self
            .transport
            .command(self.lun, &command, BotDataPhase::Out(data))
            .map_err(ScsiError::Transport)?;
        require_passed(status)
    }

    pub fn synchronize_cache_10(&mut self) -> Result<(), ScsiError<P::Error>> {
        let command = [SYNCHRONIZE_CACHE_10_OPCODE, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let status = self
            .transport
            .command(self.lun, &command, BotDataPhase::None)
            .map_err(ScsiError::Transport)?;
        require_passed(status)
    }

    fn validate_transfer(
        &self,
        start_lba: u32,
        block_count: u16,
        byte_length: usize,
    ) -> Result<(), ScsiError<P::Error>> {
        let capacity = self.capacity.ok_or(ScsiError::CapacityUnknown)?;
        if block_count == 0 {
            return Err(ScsiError::InvalidTransferLength);
        }
        let expected_length = usize::try_from(capacity.block_size)
            .ok()
            .and_then(|size| size.checked_mul(usize::from(block_count)))
            .ok_or(ScsiError::InvalidTransferLength)?;
        if byte_length != expected_length {
            return Err(ScsiError::InvalidTransferLength);
        }
        let last_lba = start_lba
            .checked_add(u32::from(block_count) - 1)
            .ok_or(ScsiError::AddressOutOfRange)?;
        if last_lba > capacity.last_lba {
            return Err(ScsiError::AddressOutOfRange);
        }
        Ok(())
    }

    pub fn capacity(&self) -> Option<ScsiCapacity> {
        self.capacity
    }

    pub fn into_transport(self) -> BulkOnlyTransport<P> {
        self.transport
    }
}

fn require_passed<E>(status: BotCommandStatus) -> Result<(), ScsiError<E>> {
    match status {
        BotCommandStatus::Passed => Ok(()),
        BotCommandStatus::CommandFailed { residue } => Err(ScsiError::CheckCondition { residue }),
    }
}

fn trimmed_ascii(bytes: &[u8]) -> &str {
    let end = bytes
        .iter()
        .rposition(|byte| *byte != b' ' && *byte != 0)
        .map(|index| index + 1)
        .unwrap_or(0);
    core::str::from_utf8(&bytes[..end]).unwrap_or("")
}

fn read_be_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::{
        ScsiError, UsbMassStorageBlockDevice, UsbMassStorageWritableBlockDevice, UsbScsiDevice,
    };
    use crate::drivers::usb_bulk_only::{
        BulkOnlyTransport, CBW_LENGTH, CSW_LENGTH, CSW_SIGNATURE, UsbBulkPipe,
    };
    use crate::storage::block::{BLOCK_SIZE, BlockDevice, BlockError};

    struct FakeScsiPipe {
        response: [u8; 512],
        response_len: usize,
        command_opcode: u8,
        command: [u8; 16],
        command_len: usize,
        tag: u32,
        command_failed: bool,
        data_out: [u8; 512],
        data_out_len: usize,
    }

    impl FakeScsiPipe {
        fn new(response: &[u8]) -> Self {
            let mut stored_response = [0u8; 512];
            stored_response[..response.len()].copy_from_slice(response);
            Self {
                response: stored_response,
                response_len: response.len(),
                command_opcode: 0xff,
                command: [0; 16],
                command_len: 0,
                tag: 0,
                command_failed: false,
                data_out: [0; 512],
                data_out_len: 0,
            }
        }
    }

    impl UsbBulkPipe for FakeScsiPipe {
        type Error = ();

        fn bulk_out(&mut self, endpoint: u8, bytes: &[u8]) -> Result<usize, Self::Error> {
            assert_eq!(endpoint, 0x02);
            if bytes.len() == CBW_LENGTH {
                self.tag = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
                self.command_opcode = bytes[15];
                self.command_len = usize::from(bytes[14]);
                self.command[..self.command_len].copy_from_slice(&bytes[15..15 + self.command_len]);
            } else {
                assert!(bytes.len() <= self.data_out.len());
                self.data_out[..bytes.len()].copy_from_slice(bytes);
                self.data_out_len = bytes.len();
            }
            Ok(bytes.len())
        }

        fn bulk_in(&mut self, endpoint: u8, bytes: &mut [u8]) -> Result<usize, Self::Error> {
            assert_eq!(endpoint, 0x81);
            if bytes.len() != CSW_LENGTH {
                assert_eq!(bytes.len(), self.response_len);
                bytes.copy_from_slice(&self.response[..self.response_len]);
            } else {
                assert_eq!(bytes.len(), CSW_LENGTH);
                bytes.fill(0);
                bytes[0..4].copy_from_slice(&CSW_SIGNATURE.to_le_bytes());
                bytes[4..8].copy_from_slice(&self.tag.to_le_bytes());
                bytes[12] = self.command_failed as u8;
            }
            Ok(bytes.len())
        }

        fn reset_recovery(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test_case]
    fn inquiry_parses_direct_access_device_identity() {
        let mut response = [0u8; 36];
        response[0] = 0;
        response[1] = 0x80;
        response[2] = 6;
        response[3] = 2;
        response[4] = 31;
        response[8..16].copy_from_slice(b"VANTARA ");
        response[16..32].copy_from_slice(b"USB DISK        ");
        response[32..36].copy_from_slice(b"1.0 ");
        let pipe = FakeScsiPipe::new(&response);
        let transport = BulkOnlyTransport::new(pipe, 0x81, 0x02);
        let mut device = UsbScsiDevice::new(transport, 0);

        let inquiry = device.inquiry().unwrap();
        assert_eq!(inquiry.peripheral_device_type, 0);
        assert!(inquiry.removable);
        assert_eq!(inquiry.vendor_str(), "VANTARA");
        assert_eq!(inquiry.product_str(), "USB DISK");
        assert_eq!(inquiry.revision_str(), "1.0");
        assert_eq!(device.into_transport().into_pipe().command_opcode, 0x12);
    }

    #[test_case]
    fn test_unit_ready_uses_no_data_phase() {
        let pipe = FakeScsiPipe::new(&[]);
        let transport = BulkOnlyTransport::new(pipe, 0x81, 0x02);
        let mut device = UsbScsiDevice::new(transport, 0);

        assert_eq!(device.test_unit_ready(), Ok(()));
        assert_eq!(device.into_transport().into_pipe().command_opcode, 0x00);
    }

    #[test_case]
    fn reports_scsi_check_condition() {
        let mut pipe = FakeScsiPipe::new(&[]);
        pipe.command_failed = true;
        let transport = BulkOnlyTransport::new(pipe, 0x81, 0x02);
        let mut device = UsbScsiDevice::new(transport, 0);

        assert_eq!(
            device.test_unit_ready(),
            Err(ScsiError::CheckCondition { residue: 0 })
        );
    }

    #[test_case]
    fn reads_capacity_and_one_block_with_read_10() {
        let capacity_response = [0x00, 0x00, 0x0f, 0xff, 0x00, 0x00, 0x02, 0x00];
        let pipe = FakeScsiPipe::new(&capacity_response);
        let transport = BulkOnlyTransport::new(pipe, 0x81, 0x02);
        let mut device = UsbScsiDevice::new(transport, 0);

        let capacity = device.read_capacity_10().unwrap();
        assert_eq!(capacity.last_lba, 4095);
        assert_eq!(capacity.block_count, 4096);
        assert_eq!(capacity.block_size, 512);

        let mut pipe = device.into_transport().into_pipe();
        pipe.response.fill(0xa5);
        pipe.response_len = 512;
        let transport = BulkOnlyTransport::new(pipe, 0x81, 0x02);
        let mut device = UsbScsiDevice::new(transport, 0);
        device.capacity = Some(capacity);
        let mut block = [0u8; 512];
        device.read_10(7, 1, &mut block).unwrap();
        assert_eq!(block, [0xa5; 512]);

        let pipe = device.into_transport().into_pipe();
        assert_eq!(pipe.command_opcode, 0x28);
        assert_eq!(&pipe.command[2..6], &[0, 0, 0, 7]);
        assert_eq!(&pipe.command[7..9], &[0, 1]);
    }

    #[test_case]
    fn read_10_rejects_wrong_buffer_length_and_lba_range() {
        let pipe = FakeScsiPipe::new(&[]);
        let transport = BulkOnlyTransport::new(pipe, 0x81, 0x02);
        let mut device = UsbScsiDevice::new(transport, 0);
        device.capacity = Some(super::ScsiCapacity {
            last_lba: 9,
            block_count: 10,
            block_size: 512,
        });

        assert_eq!(
            device.read_10(0, 1, &mut [0u8; 16]),
            Err(ScsiError::InvalidTransferLength)
        );
        assert_eq!(
            device.read_10(9, 2, &mut [0u8; 1024]),
            Err(ScsiError::AddressOutOfRange)
        );
    }

    #[test_case]
    fn block_device_reads_one_sector_and_rejects_writes() {
        let mut pipe = FakeScsiPipe::new(&[0x5a; BLOCK_SIZE]);
        pipe.response_len = BLOCK_SIZE;
        let transport = BulkOnlyTransport::new(pipe, 0x81, 0x02);
        let mut scsi = UsbScsiDevice::new(transport, 0);
        scsi.capacity = Some(super::ScsiCapacity {
            last_lba: 7,
            block_count: 8,
            block_size: BLOCK_SIZE as u32,
        });
        let mut device = UsbMassStorageBlockDevice {
            device: scsi,
            blocks: 8,
        };

        let mut block = [0u8; BLOCK_SIZE];
        assert_eq!(device.read_block(3, &mut block), Ok(()));
        assert_eq!(block, [0x5a; BLOCK_SIZE]);
        assert_eq!(device.write_block(3, &block), Err(BlockError::ReadOnly));
        assert_eq!(
            device.read_block(8, &mut block),
            Err(BlockError::OutOfRange)
        );
    }

    #[test_case]
    fn parses_fixed_format_request_sense() {
        let mut response = [0u8; 18];
        response[0] = 0xf0;
        response[2] = 0x06;
        response[12] = 0x29;
        response[13] = 0x00;
        let pipe = FakeScsiPipe::new(&response);
        let transport = BulkOnlyTransport::new(pipe, 0x81, 0x02);
        let mut device = UsbScsiDevice::new(transport, 0);

        let sense = device.request_sense().unwrap();
        assert!(sense.valid);
        assert_eq!(sense.response_code, 0x70);
        assert_eq!(sense.sense_key, 0x06);
        assert_eq!(sense.additional_sense_code, 0x29);
        assert_eq!(sense.additional_sense_qualifier, 0);
    }

    #[test_case]
    fn writable_block_device_writes_then_flushes() {
        let pipe = FakeScsiPipe::new(&[]);
        let transport = BulkOnlyTransport::new(pipe, 0x81, 0x02);
        let mut scsi = UsbScsiDevice::new(transport, 0);
        scsi.capacity = Some(super::ScsiCapacity {
            last_lba: 7,
            block_count: 8,
            block_size: BLOCK_SIZE as u32,
        });
        let mut device = UsbMassStorageWritableBlockDevice {
            device: scsi,
            blocks: 8,
        };
        let block = [0xa6; BLOCK_SIZE];

        assert_eq!(device.write_block(2, &block), Ok(()));
        let pipe = device.into_scsi_device().into_transport().into_pipe();
        assert_eq!(pipe.command_opcode, 0x35);
        assert_eq!(pipe.data_out_len, BLOCK_SIZE);
        assert_eq!(pipe.data_out, block);
    }
}
