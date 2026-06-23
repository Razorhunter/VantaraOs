use x86_64::instructions::port::Port;

use super::block::{BLOCK_SIZE, BlockDevice, BlockError};

const DATA: u16 = 0x1f0;
const SECTOR_COUNT: u16 = 0x1f2;
const LBA_LOW: u16 = 0x1f3;
const LBA_MID: u16 = 0x1f4;
const LBA_HIGH: u16 = 0x1f5;
const DRIVE: u16 = 0x1f6;
const STATUS_COMMAND: u16 = 0x1f7;
const ALT_STATUS_CONTROL: u16 = 0x3f6;

const STATUS_ERROR: u8 = 1 << 0;
const STATUS_DATA_REQUEST: u8 = 1 << 3;
const STATUS_DEVICE_FAULT: u8 = 1 << 5;
const STATUS_BUSY: u8 = 1 << 7;

const COMMAND_READ_SECTORS: u8 = 0x20;
const COMMAND_WRITE_SECTORS: u8 = 0x30;
const COMMAND_CACHE_FLUSH: u8 = 0xe7;
const POLL_LIMIT: usize = 1_000_000;

pub struct AtaPioDisk {
    slave: bool,
    blocks: u64,
}

impl AtaPioDisk {
    pub const fn primary_slave(blocks: u64) -> Self {
        Self {
            slave: true,
            blocks,
        }
    }

    pub fn probe(&self) -> Result<(), BlockError> {
        // SAFETY: these fixed legacy ATA ports belong to the primary IDE
        // channel, and the disk lock serializes all command-register access.
        unsafe {
            // This driver is deliberately polling-only. nIEN prevents the
            // device from raising IRQ14 for commands completed below.
            Port::<u8>::new(ALT_STATUS_CONTROL).write(0x02);
            Port::<u8>::new(DRIVE).write(self.drive_head(0));
            self.io_delay();
            let status = Port::<u8>::new(STATUS_COMMAND).read();
            if status == 0 || status == 0xff {
                return Err(BlockError::NoDevice);
            }
        }
        self.wait_ready().map(|_| ())
    }

    fn transfer_setup(&self, block: u64, command: u8) -> Result<(), BlockError> {
        if block >= self.blocks || block > 0x0fff_ffff {
            return Err(BlockError::OutOfRange);
        }
        let lba = block as u32;
        self.wait_ready()?;
        // SAFETY: the primary IDE command ports are accessed while holding the
        // owning disk lock, and the validated LBA fits the 28-bit registers.
        unsafe {
            Port::<u8>::new(DRIVE).write(self.drive_head(lba));
            Port::<u8>::new(SECTOR_COUNT).write(1);
            Port::<u8>::new(LBA_LOW).write(lba as u8);
            Port::<u8>::new(LBA_MID).write((lba >> 8) as u8);
            Port::<u8>::new(LBA_HIGH).write((lba >> 16) as u8);
            Port::<u8>::new(STATUS_COMMAND).write(command);
        }
        self.wait_data().map(|_| ())
    }

    fn drive_head(&self, lba: u32) -> u8 {
        0xe0 | ((self.slave as u8) << 4) | ((lba >> 24) as u8 & 0x0f)
    }

    fn wait_ready(&self) -> Result<u8, BlockError> {
        self.poll_status(false)
    }

    fn wait_data(&self) -> Result<u8, BlockError> {
        self.poll_status(true)
    }

    fn poll_status(&self, require_data: bool) -> Result<u8, BlockError> {
        for _ in 0..POLL_LIMIT {
            // SAFETY: status reads are side-effect-safe ATA polling operations
            // serialized by the owning disk lock.
            let status = unsafe { Port::<u8>::new(STATUS_COMMAND).read() };
            if status == 0 || status == 0xff {
                return Err(BlockError::NoDevice);
            }
            if status & STATUS_DEVICE_FAULT != 0 {
                return Err(BlockError::DeviceFault);
            }
            if status & STATUS_ERROR != 0 {
                return Err(BlockError::DeviceFault);
            }
            if status & STATUS_BUSY == 0 && (!require_data || status & STATUS_DATA_REQUEST != 0) {
                return Ok(status);
            }
            core::hint::spin_loop();
        }
        Err(BlockError::Timeout)
    }

    fn io_delay(&self) {
        // SAFETY: four alternate-status reads are the specified ATA 400 ns
        // delay and do not acknowledge or mutate device state.
        unsafe {
            let mut port = Port::<u8>::new(ALT_STATUS_CONTROL);
            for _ in 0..4 {
                let _ = port.read();
            }
        }
    }
}

impl BlockDevice for AtaPioDisk {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(
        &mut self,
        block_index: u64,
        buffer: &mut [u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        self.transfer_setup(block_index, COMMAND_READ_SECTORS)?;
        // SAFETY: DRQ is set before this transfer, and exactly one 512-byte
        // sector is consumed as 256 data-port words.
        unsafe {
            let mut data = Port::<u16>::new(DATA);
            for (index, chunk) in buffer.chunks_exact_mut(2).enumerate() {
                let word = data.read().to_le_bytes();
                chunk.copy_from_slice(&word);
                debug_assert!(index < BLOCK_SIZE / 2);
            }
        }
        self.io_delay();
        Ok(())
    }

    fn write_block(
        &mut self,
        block_index: u64,
        buffer: &[u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        self.transfer_setup(block_index, COMMAND_WRITE_SECTORS)?;
        // SAFETY: DRQ is set before this transfer, exactly one sector is
        // emitted, and the cache-flush command is sent on the same channel.
        unsafe {
            let mut data = Port::<u16>::new(DATA);
            for chunk in buffer.chunks_exact(2) {
                data.write(u16::from_le_bytes([chunk[0], chunk[1]]));
            }
            Port::<u8>::new(STATUS_COMMAND).write(COMMAND_CACHE_FLUSH);
        }
        self.wait_ready()?;
        self.io_delay();
        Ok(())
    }
}
