pub const BLOCK_SIZE: usize = 512;

pub trait BlockDevice {
    fn block_count(&self) -> u64;
    fn read_block(&self, block_index: u64, buffer: &mut [u8; BLOCK_SIZE])
    -> Result<(), BlockError>;
    fn write_block(
        &mut self,
        block_index: u64,
        buffer: &[u8; BLOCK_SIZE],
    ) -> Result<(), BlockError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    OutOfRange,
    ReadOnly,
}
