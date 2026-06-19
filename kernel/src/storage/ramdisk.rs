use super::block::{BLOCK_SIZE, BlockDevice, BlockError};

pub struct RamDisk {
    data: &'static [u8],
}

impl RamDisk {
    pub const fn new(data: &'static [u8]) -> Self {
        Self { data }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }
}

impl BlockDevice for RamDisk {
    fn block_count(&self) -> u64 {
        self.data.len().div_ceil(BLOCK_SIZE) as u64
    }

    fn read_block(
        &self,
        block_index: u64,
        buffer: &mut [u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        if block_index >= self.block_count() {
            return Err(BlockError::OutOfRange);
        }

        let start = block_index as usize * BLOCK_SIZE;
        let end = (start + BLOCK_SIZE).min(self.data.len());
        let len = end.saturating_sub(start);

        buffer.fill(0);
        buffer[..len].copy_from_slice(&self.data[start..end]);
        Ok(())
    }

    fn write_block(
        &mut self,
        _block_index: u64,
        _buffer: &[u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        Err(BlockError::ReadOnly)
    }
}
