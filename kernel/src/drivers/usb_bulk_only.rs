pub const CBW_LENGTH: usize = 31;
pub const CSW_LENGTH: usize = 13;
pub const CBW_SIGNATURE: u32 = 0x4342_5355;
pub const CSW_SIGNATURE: u32 = 0x5342_5355;
const DATA_IN_FLAG: u8 = 0x80;
const MAX_LUN: u8 = 15;
const MAX_CDB_LENGTH: usize = 16;

pub trait UsbBulkPipe {
    type Error;

    fn bulk_out(&mut self, endpoint: u8, bytes: &[u8]) -> Result<usize, Self::Error>;
    fn bulk_in(&mut self, endpoint: u8, bytes: &mut [u8]) -> Result<usize, Self::Error>;
    fn reset_recovery(&mut self) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotCommandStatus {
    Passed,
    CommandFailed { residue: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotError<E> {
    InvalidLun,
    InvalidCommandBlock,
    TransferTooLarge,
    Pipe(E),
    ShortTransfer,
    InvalidCswSignature,
    TagMismatch,
    InvalidResidue,
    PhaseError,
    InvalidCswStatus,
}

pub enum BotDataPhase<'a> {
    None,
    In(&'a mut [u8]),
    Out(&'a [u8]),
}

pub struct BulkOnlyTransport<P> {
    pipe: P,
    bulk_in_endpoint: u8,
    bulk_out_endpoint: u8,
    next_tag: u32,
}

impl<P: UsbBulkPipe> BulkOnlyTransport<P> {
    pub const fn new(pipe: P, bulk_in_endpoint: u8, bulk_out_endpoint: u8) -> Self {
        Self {
            pipe,
            bulk_in_endpoint,
            bulk_out_endpoint,
            next_tag: 1,
        }
    }

    pub fn command(
        &mut self,
        lun: u8,
        command_block: &[u8],
        data: BotDataPhase<'_>,
    ) -> Result<BotCommandStatus, BotError<P::Error>> {
        if lun > MAX_LUN {
            return Err(BotError::InvalidLun);
        }
        if command_block.is_empty() || command_block.len() > MAX_CDB_LENGTH {
            return Err(BotError::InvalidCommandBlock);
        }

        let (transfer_length, flags) = match &data {
            BotDataPhase::None => (0, 0),
            BotDataPhase::In(bytes) => (bytes.len(), DATA_IN_FLAG),
            BotDataPhase::Out(bytes) => (bytes.len(), 0),
        };
        let transfer_length =
            u32::try_from(transfer_length).map_err(|_| BotError::TransferTooLarge)?;
        let tag = self.allocate_tag();
        let cbw = encode_cbw(tag, transfer_length, flags, lun, command_block);
        if let Err(error) = self.write_exact(self.bulk_out_endpoint, &cbw) {
            let _ = self.pipe.reset_recovery();
            return Err(error);
        }

        match data {
            BotDataPhase::None => {}
            BotDataPhase::In(bytes) => {
                if let Err(error) = self.read_exact(self.bulk_in_endpoint, bytes) {
                    let _ = self.pipe.reset_recovery();
                    return Err(error);
                }
            }
            BotDataPhase::Out(bytes) => {
                if let Err(error) = self.write_exact(self.bulk_out_endpoint, bytes) {
                    let _ = self.pipe.reset_recovery();
                    return Err(error);
                }
            }
        }

        let mut csw = [0u8; CSW_LENGTH];
        if let Err(error) = self.read_exact(self.bulk_in_endpoint, &mut csw) {
            let _ = self.pipe.reset_recovery();
            return Err(error);
        }
        let result = parse_csw(&csw, tag, transfer_length);
        if matches!(result, Err(BotError::PhaseError)) {
            let _ = self.pipe.reset_recovery();
        }
        result
    }

    pub fn into_pipe(self) -> P {
        self.pipe
    }

    fn allocate_tag(&mut self) -> u32 {
        let tag = self.next_tag;
        self.next_tag = self.next_tag.wrapping_add(1);
        if self.next_tag == 0 {
            self.next_tag = 1;
        }
        tag
    }

    fn write_exact(&mut self, endpoint: u8, bytes: &[u8]) -> Result<(), BotError<P::Error>> {
        let written = self
            .pipe
            .bulk_out(endpoint, bytes)
            .map_err(BotError::Pipe)?;
        if written != bytes.len() {
            return Err(BotError::ShortTransfer);
        }
        Ok(())
    }

    fn read_exact(&mut self, endpoint: u8, bytes: &mut [u8]) -> Result<(), BotError<P::Error>> {
        let read = self.pipe.bulk_in(endpoint, bytes).map_err(BotError::Pipe)?;
        if read != bytes.len() {
            return Err(BotError::ShortTransfer);
        }
        Ok(())
    }
}

fn encode_cbw(
    tag: u32,
    transfer_length: u32,
    flags: u8,
    lun: u8,
    command_block: &[u8],
) -> [u8; CBW_LENGTH] {
    let mut bytes = [0u8; CBW_LENGTH];
    bytes[0..4].copy_from_slice(&CBW_SIGNATURE.to_le_bytes());
    bytes[4..8].copy_from_slice(&tag.to_le_bytes());
    bytes[8..12].copy_from_slice(&transfer_length.to_le_bytes());
    bytes[12] = flags;
    bytes[13] = lun;
    bytes[14] = command_block.len() as u8;
    bytes[15..15 + command_block.len()].copy_from_slice(command_block);
    bytes
}

fn parse_csw<E>(
    bytes: &[u8; CSW_LENGTH],
    expected_tag: u32,
    transfer_length: u32,
) -> Result<BotCommandStatus, BotError<E>> {
    if read_u32(bytes, 0) != CSW_SIGNATURE {
        return Err(BotError::InvalidCswSignature);
    }
    if read_u32(bytes, 4) != expected_tag {
        return Err(BotError::TagMismatch);
    }
    let residue = read_u32(bytes, 8);
    if residue > transfer_length {
        return Err(BotError::InvalidResidue);
    }
    match bytes[12] {
        0 if residue == 0 => Ok(BotCommandStatus::Passed),
        0 => Err(BotError::InvalidResidue),
        1 => Ok(BotCommandStatus::CommandFailed { residue }),
        2 => Err(BotError::PhaseError),
        _ => Err(BotError::InvalidCswStatus),
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::{
        BotCommandStatus, BotDataPhase, BotError, BulkOnlyTransport, CBW_LENGTH, CBW_SIGNATURE,
        CSW_LENGTH, CSW_SIGNATURE, UsbBulkPipe,
    };

    struct FakeBulkPipe {
        expected_out_endpoint: u8,
        expected_in_endpoint: u8,
        cbw: [u8; CBW_LENGTH],
        cbw_seen: bool,
        data_out: [u8; 4],
        data_out_seen: bool,
        data_in: [u8; 4],
        csw: [u8; CSW_LENGTH],
        in_phase: usize,
        reset_count: usize,
    }

    impl FakeBulkPipe {
        fn passing(data_in: [u8; 4]) -> Self {
            let mut csw = [0u8; CSW_LENGTH];
            csw[0..4].copy_from_slice(&CSW_SIGNATURE.to_le_bytes());
            csw[4..8].copy_from_slice(&1u32.to_le_bytes());
            Self {
                expected_out_endpoint: 0x02,
                expected_in_endpoint: 0x81,
                cbw: [0; CBW_LENGTH],
                cbw_seen: false,
                data_out: [0; 4],
                data_out_seen: false,
                data_in,
                csw,
                in_phase: 0,
                reset_count: 0,
            }
        }
    }

    impl UsbBulkPipe for FakeBulkPipe {
        type Error = ();

        fn bulk_out(&mut self, endpoint: u8, bytes: &[u8]) -> Result<usize, Self::Error> {
            assert_eq!(endpoint, self.expected_out_endpoint);
            if !self.cbw_seen {
                assert_eq!(bytes.len(), CBW_LENGTH);
                self.cbw.copy_from_slice(bytes);
                self.cbw_seen = true;
            } else {
                assert_eq!(bytes.len(), self.data_out.len());
                self.data_out.copy_from_slice(bytes);
                self.data_out_seen = true;
            }
            Ok(bytes.len())
        }

        fn bulk_in(&mut self, endpoint: u8, bytes: &mut [u8]) -> Result<usize, Self::Error> {
            assert_eq!(endpoint, self.expected_in_endpoint);
            if bytes.len() == self.data_in.len() && self.in_phase == 0 {
                bytes.copy_from_slice(&self.data_in);
            } else {
                assert_eq!(bytes.len(), CSW_LENGTH);
                bytes.copy_from_slice(&self.csw);
            }
            self.in_phase += 1;
            Ok(bytes.len())
        }

        fn reset_recovery(&mut self) -> Result<(), Self::Error> {
            self.reset_count += 1;
            Ok(())
        }
    }

    #[test_case]
    fn performs_cbw_data_in_and_csw_transaction() {
        let pipe = FakeBulkPipe::passing([1, 2, 3, 4]);
        let mut transport = BulkOnlyTransport::new(pipe, 0x81, 0x02);
        let mut data = [0u8; 4];

        assert_eq!(
            transport.command(0, &[0x12, 0, 0, 0, 4, 0], BotDataPhase::In(&mut data)),
            Ok(BotCommandStatus::Passed)
        );
        assert_eq!(data, [1, 2, 3, 4]);

        let pipe = transport.into_pipe();
        assert_eq!(
            u32::from_le_bytes(pipe.cbw[0..4].try_into().unwrap()),
            CBW_SIGNATURE
        );
        assert_eq!(u32::from_le_bytes(pipe.cbw[4..8].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(pipe.cbw[8..12].try_into().unwrap()), 4);
        assert_eq!(pipe.cbw[12], 0x80);
        assert_eq!(pipe.cbw[14], 6);
        assert_eq!(&pipe.cbw[15..21], &[0x12, 0, 0, 0, 4, 0]);
    }

    #[test_case]
    fn performs_data_out_before_csw() {
        let pipe = FakeBulkPipe::passing([0; 4]);
        let mut transport = BulkOnlyTransport::new(pipe, 0x81, 0x02);

        assert_eq!(
            transport.command(0, &[0x2a], BotDataPhase::Out(&[9, 8, 7, 6])),
            Ok(BotCommandStatus::Passed)
        );
        let pipe = transport.into_pipe();
        assert!(pipe.data_out_seen);
        assert_eq!(pipe.data_out, [9, 8, 7, 6]);
        assert_eq!(pipe.cbw[12], 0);
    }

    #[test_case]
    fn rejects_csw_with_wrong_tag() {
        let mut pipe = FakeBulkPipe::passing([0; 4]);
        pipe.csw[4..8].copy_from_slice(&99u32.to_le_bytes());
        let mut transport = BulkOnlyTransport::new(pipe, 0x81, 0x02);

        assert_eq!(
            transport.command(0, &[0], BotDataPhase::None),
            Err(BotError::TagMismatch)
        );
    }

    #[test_case]
    fn invokes_reset_recovery_after_phase_error() {
        let mut pipe = FakeBulkPipe::passing([0; 4]);
        pipe.csw[12] = 2;
        let mut transport = BulkOnlyTransport::new(pipe, 0x81, 0x02);

        assert_eq!(
            transport.command(0, &[0], BotDataPhase::None),
            Err(BotError::PhaseError)
        );
        assert_eq!(transport.into_pipe().reset_count, 1);
    }
}
