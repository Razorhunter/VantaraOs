use super::block::{BLOCK_SIZE, BlockDevice, BlockError};

pub const MBR_PARTITION_COUNT: usize = 4;
pub const VANTARA_PARTITION_TYPE: u8 = 0x7f;
const MBR_TABLE_OFFSET: usize = 446;
const MBR_ENTRY_SIZE: usize = 16;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MbrPartition {
    pub bootable: bool,
    pub partition_type: u8,
    pub start_block: u64,
    pub block_count: u64,
}

pub fn parse_mbr(
    sector: &[u8; BLOCK_SIZE],
    device_blocks: u64,
) -> Result<[Option<MbrPartition>; MBR_PARTITION_COUNT], PartitionError> {
    if sector[510] != 0x55 || sector[511] != 0xaa {
        return Err(PartitionError::MissingSignature);
    }

    let mut partitions = [None; MBR_PARTITION_COUNT];
    for (index, slot) in partitions.iter_mut().enumerate() {
        let offset = MBR_TABLE_OFFSET + index * MBR_ENTRY_SIZE;
        let boot = sector[offset];
        if boot != 0 && boot != 0x80 {
            return Err(PartitionError::InvalidBootFlag);
        }
        let partition_type = sector[offset + 4];
        let start_block =
            u32::from_le_bytes(sector[offset + 8..offset + 12].try_into().unwrap_or([0; 4])) as u64;
        let block_count = u32::from_le_bytes(
            sector[offset + 12..offset + 16]
                .try_into()
                .unwrap_or([0; 4]),
        ) as u64;
        if partition_type == 0 || block_count == 0 {
            continue;
        }
        let end = start_block
            .checked_add(block_count)
            .ok_or(PartitionError::OutOfRange)?;
        if start_block == 0 || end > device_blocks {
            return Err(PartitionError::OutOfRange);
        }
        *slot = Some(MbrPartition {
            bootable: boot == 0x80,
            partition_type,
            start_block,
            block_count,
        });
    }
    Ok(partitions)
}

pub fn select_vantara_partition(
    partitions: &[Option<MbrPartition>; MBR_PARTITION_COUNT],
) -> Option<MbrPartition> {
    partitions
        .iter()
        .flatten()
        .find(|partition| partition.partition_type == VANTARA_PARTITION_TYPE)
        .copied()
        .or_else(|| partitions.iter().flatten().next().copied())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionError {
    MissingSignature,
    InvalidBootFlag,
    OutOfRange,
}

pub struct PartitionBlockDevice<D> {
    device: D,
    physical_blocks: u64,
    start_block: u64,
    block_count: u64,
    partition_type: u8,
    partitioned: bool,
}

impl<D> PartitionBlockDevice<D> {
    pub const fn new(device: D, physical_blocks: u64, visible_blocks: u64) -> Self {
        Self {
            device,
            physical_blocks,
            start_block: 0,
            block_count: visible_blocks,
            partition_type: 0,
            partitioned: false,
        }
    }

    pub fn device(&self) -> &D {
        &self.device
    }

    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    pub fn reset_geometry(
        &mut self,
        physical_blocks: u64,
        visible_blocks: u64,
    ) -> Result<(), PartitionError> {
        if physical_blocks == 0 || visible_blocks == 0 || visible_blocks > physical_blocks {
            return Err(PartitionError::OutOfRange);
        }
        self.physical_blocks = physical_blocks;
        self.start_block = 0;
        self.block_count = visible_blocks;
        self.partition_type = 0;
        self.partitioned = false;
        Ok(())
    }

    pub fn configure_superfloppy(&mut self, block_count: u64) -> Result<(), PartitionError> {
        self.configure(0, block_count, 0, false)
    }

    pub fn configure_partition(&mut self, partition: MbrPartition) -> Result<(), PartitionError> {
        self.configure(
            partition.start_block,
            partition.block_count,
            partition.partition_type,
            true,
        )
    }

    fn configure(
        &mut self,
        start_block: u64,
        block_count: u64,
        partition_type: u8,
        partitioned: bool,
    ) -> Result<(), PartitionError> {
        let end = start_block
            .checked_add(block_count)
            .ok_or(PartitionError::OutOfRange)?;
        if block_count == 0 || end > self.physical_blocks {
            return Err(PartitionError::OutOfRange);
        }
        self.start_block = start_block;
        self.block_count = block_count;
        self.partition_type = partition_type;
        self.partitioned = partitioned;
        Ok(())
    }

    pub const fn start_block(&self) -> u64 {
        self.start_block
    }

    pub const fn partition_type(&self) -> u8 {
        self.partition_type
    }

    pub const fn partitioned(&self) -> bool {
        self.partitioned
    }
}

impl<D: BlockDevice> BlockDevice for PartitionBlockDevice<D> {
    fn block_count(&self) -> u64 {
        self.block_count
    }

    fn read_block(
        &mut self,
        block_index: u64,
        buffer: &mut [u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        if block_index >= self.block_count {
            crate::serial_println!(
                "[PARTITION-IO] read out-of-range lba={} start={} blocks={} physical={}",
                block_index,
                self.start_block,
                self.block_count,
                self.physical_blocks
            );
            return Err(BlockError::OutOfRange);
        }
        self.device
            .read_block(self.start_block + block_index, buffer)
    }

    fn write_block(
        &mut self,
        block_index: u64,
        buffer: &[u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        if block_index >= self.block_count {
            return Err(BlockError::OutOfRange);
        }
        self.device
            .write_block(self.start_block + block_index, buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MbrPartition, PartitionBlockDevice, PartitionError, VANTARA_PARTITION_TYPE, parse_mbr,
        select_vantara_partition,
    };
    use crate::storage::block::{BLOCK_SIZE, BlockDevice, BlockError};

    struct MemoryDisk {
        blocks: [[u8; BLOCK_SIZE]; 8],
    }

    impl BlockDevice for MemoryDisk {
        fn block_count(&self) -> u64 {
            8
        }

        fn read_block(
            &mut self,
            block_index: u64,
            buffer: &mut [u8; BLOCK_SIZE],
        ) -> Result<(), BlockError> {
            *buffer = self.blocks[block_index as usize];
            Ok(())
        }

        fn write_block(
            &mut self,
            block_index: u64,
            buffer: &[u8; BLOCK_SIZE],
        ) -> Result<(), BlockError> {
            self.blocks[block_index as usize] = *buffer;
            Ok(())
        }
    }

    fn mbr_entry(sector: &mut [u8; BLOCK_SIZE], index: usize, kind: u8, start: u32, count: u32) {
        let offset = 446 + index * 16;
        sector[offset + 4] = kind;
        sector[offset + 8..offset + 12].copy_from_slice(&start.to_le_bytes());
        sector[offset + 12..offset + 16].copy_from_slice(&count.to_le_bytes());
        sector[510] = 0x55;
        sector[511] = 0xaa;
    }

    #[test_case]
    fn parses_and_prefers_vantara_partition() {
        let mut sector = [0; BLOCK_SIZE];
        mbr_entry(&mut sector, 0, 0x83, 1, 2);
        mbr_entry(&mut sector, 1, VANTARA_PARTITION_TYPE, 3, 4);
        let partitions = parse_mbr(&sector, 8).unwrap();
        assert_eq!(
            select_vantara_partition(&partitions),
            Some(MbrPartition {
                bootable: false,
                partition_type: VANTARA_PARTITION_TYPE,
                start_block: 3,
                block_count: 4,
            })
        );
    }

    #[test_case]
    fn rejects_partition_past_device_end() {
        let mut sector = [0; BLOCK_SIZE];
        mbr_entry(&mut sector, 0, VANTARA_PARTITION_TYPE, 7, 2);
        assert_eq!(parse_mbr(&sector, 8), Err(PartitionError::OutOfRange));
    }

    #[test_case]
    fn partition_view_translates_block_numbers() {
        let disk = MemoryDisk {
            blocks: [[0; BLOCK_SIZE]; 8],
        };
        let mut partition = PartitionBlockDevice::new(disk, 8, 8);
        partition
            .configure_partition(MbrPartition {
                bootable: false,
                partition_type: VANTARA_PARTITION_TYPE,
                start_block: 2,
                block_count: 4,
            })
            .unwrap();
        partition.write_block(1, &[0x5a; BLOCK_SIZE]).unwrap();
        assert_eq!(partition.device().blocks[3], [0x5a; BLOCK_SIZE]);
        assert_eq!(
            partition.write_block(4, &[0; BLOCK_SIZE]),
            Err(BlockError::OutOfRange)
        );
    }
}
