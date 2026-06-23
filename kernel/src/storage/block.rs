pub const BLOCK_SIZE: usize = 512;

pub trait BlockDevice {
    fn block_count(&self) -> u64;
    fn read_block(
        &mut self,
        block_index: u64,
        buffer: &mut [u8; BLOCK_SIZE],
    ) -> Result<(), BlockError>;
    fn write_block(
        &mut self,
        block_index: u64,
        buffer: &[u8; BLOCK_SIZE],
    ) -> Result<(), BlockError>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlockCacheStats {
    pub read_hits: u64,
    pub read_misses: u64,
    pub writes: u64,
    pub evictions: u64,
}

#[derive(Clone, Copy)]
struct CacheEntry {
    valid: bool,
    block_index: u64,
    last_used: u64,
    data: [u8; BLOCK_SIZE],
}

impl CacheEntry {
    const EMPTY: Self = Self {
        valid: false,
        block_index: 0,
        last_used: 0,
        data: [0; BLOCK_SIZE],
    };
}

pub struct CachedBlockDevice<D, const ENTRIES: usize> {
    device: D,
    entries: [CacheEntry; ENTRIES],
    clock: u64,
    stats: BlockCacheStats,
}

impl<D, const ENTRIES: usize> CachedBlockDevice<D, ENTRIES> {
    pub const fn new(device: D) -> Self {
        Self {
            device,
            entries: [CacheEntry::EMPTY; ENTRIES],
            clock: 0,
            stats: BlockCacheStats {
                read_hits: 0,
                read_misses: 0,
                writes: 0,
                evictions: 0,
            },
        }
    }

    pub fn backing(&self) -> &D {
        &self.device
    }

    pub fn backing_mut(&mut self) -> &mut D {
        &mut self.device
    }

    pub fn stats(&self) -> BlockCacheStats {
        self.stats
    }

    pub const fn capacity(&self) -> usize {
        ENTRIES
    }

    fn touch(&mut self) -> u64 {
        self.clock = self.clock.saturating_add(1);
        self.clock
    }

    fn hit_index(&self, block_index: u64) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.valid && entry.block_index == block_index)
    }

    fn victim_index(&self) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| !entry.valid)
            .or_else(|| {
                self.entries
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, entry)| entry.last_used)
                    .map(|(index, _)| index)
            })
    }

    fn store(&mut self, block_index: u64, data: &[u8; BLOCK_SIZE]) {
        let Some(index) = self.hit_index(block_index).or_else(|| self.victim_index()) else {
            return;
        };
        if self.entries[index].valid && self.entries[index].block_index != block_index {
            self.stats.evictions = self.stats.evictions.saturating_add(1);
        }
        let last_used = self.touch();
        self.entries[index] = CacheEntry {
            valid: true,
            block_index,
            last_used,
            data: *data,
        };
    }
}

impl<D: BlockDevice, const ENTRIES: usize> BlockDevice for CachedBlockDevice<D, ENTRIES> {
    fn block_count(&self) -> u64 {
        self.device.block_count()
    }

    fn read_block(
        &mut self,
        block_index: u64,
        buffer: &mut [u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        if block_index >= self.block_count() {
            return Err(BlockError::OutOfRange);
        }
        if let Some(index) = self.hit_index(block_index) {
            let last_used = self.touch();
            self.entries[index].last_used = last_used;
            *buffer = self.entries[index].data;
            self.stats.read_hits = self.stats.read_hits.saturating_add(1);
            return Ok(());
        }

        self.stats.read_misses = self.stats.read_misses.saturating_add(1);
        self.device.read_block(block_index, buffer)?;
        self.store(block_index, buffer);
        Ok(())
    }

    fn write_block(
        &mut self,
        block_index: u64,
        buffer: &[u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        if block_index >= self.block_count() {
            return Err(BlockError::OutOfRange);
        }
        self.device.write_block(block_index, buffer)?;
        self.stats.writes = self.stats.writes.saturating_add(1);
        self.store(block_index, buffer);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    OutOfRange,
    ReadOnly,
    NoDevice,
    Timeout,
    DeviceFault,
}

#[cfg(test)]
mod tests {
    use super::{BLOCK_SIZE, BlockDevice, BlockError, CachedBlockDevice};

    struct MemoryDisk {
        blocks: [[u8; BLOCK_SIZE]; 4],
        reads: u64,
        writes: u64,
    }

    impl MemoryDisk {
        const fn new() -> Self {
            Self {
                blocks: [[0; BLOCK_SIZE]; 4],
                reads: 0,
                writes: 0,
            }
        }
    }

    impl BlockDevice for MemoryDisk {
        fn block_count(&self) -> u64 {
            self.blocks.len() as u64
        }

        fn read_block(
            &mut self,
            block_index: u64,
            buffer: &mut [u8; BLOCK_SIZE],
        ) -> Result<(), BlockError> {
            let block = self
                .blocks
                .get(block_index as usize)
                .ok_or(BlockError::OutOfRange)?;
            self.reads += 1;
            *buffer = *block;
            Ok(())
        }

        fn write_block(
            &mut self,
            block_index: u64,
            buffer: &[u8; BLOCK_SIZE],
        ) -> Result<(), BlockError> {
            let block = self
                .blocks
                .get_mut(block_index as usize)
                .ok_or(BlockError::OutOfRange)?;
            self.writes += 1;
            *block = *buffer;
            Ok(())
        }
    }

    #[test_case]
    fn repeated_read_hits_cache() {
        let mut cache = CachedBlockDevice::<_, 2>::new(MemoryDisk::new());
        let mut block = [0; BLOCK_SIZE];
        cache.read_block(1, &mut block).unwrap();
        cache.read_block(1, &mut block).unwrap();
        assert_eq!(cache.stats().read_misses, 1);
        assert_eq!(cache.stats().read_hits, 1);
        assert_eq!(cache.backing().reads, 1);
    }

    #[test_case]
    fn write_through_updates_disk_and_cached_copy() {
        let mut cache = CachedBlockDevice::<_, 2>::new(MemoryDisk::new());
        let block = [0x5a; BLOCK_SIZE];
        cache.write_block(2, &block).unwrap();
        let mut out = [0; BLOCK_SIZE];
        cache.read_block(2, &mut out).unwrap();
        assert_eq!(out, block);
        assert_eq!(cache.stats().writes, 1);
        assert_eq!(cache.stats().read_hits, 1);
        assert_eq!(cache.backing().writes, 1);
        assert_eq!(cache.backing().reads, 0);
    }

    #[test_case]
    fn least_recently_used_entry_is_evicted() {
        let mut cache = CachedBlockDevice::<_, 2>::new(MemoryDisk::new());
        let mut block = [0; BLOCK_SIZE];
        cache.read_block(0, &mut block).unwrap();
        cache.read_block(1, &mut block).unwrap();
        cache.read_block(0, &mut block).unwrap();
        cache.read_block(2, &mut block).unwrap();
        cache.read_block(1, &mut block).unwrap();
        assert_eq!(cache.stats().evictions, 2);
        assert_eq!(cache.stats().read_misses, 4);
    }
}
