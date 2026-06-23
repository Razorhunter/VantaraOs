use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use super::{BackendStat, FileType, FilesystemBackend, FsError, append_line};
use crate::storage::ata::AtaPioDisk;
use crate::storage::block::{BLOCK_SIZE, BlockDevice, CachedBlockDevice};
use crate::storage::partition::{
    PartitionBlockDevice, PartitionError, parse_mbr, select_vantara_partition,
};
use crate::sync::PreemptMutex;

const VOLUME_BLOCKS: u64 = 2048;
const ATA_ADDRESSABLE_BLOCKS: u64 = 4096;
const MAGIC: &[u8; 8] = b"VANTFS01";
const VERSION: u32 = 1;
const HEADER_BLOCK: u64 = 0;
const DIRECTORY_START_BLOCK: u64 = 1;
const DIRECTORY_BLOCKS: u64 = 4;
const DATA_START_BLOCK: u64 = DIRECTORY_START_BLOCK + DIRECTORY_BLOCKS;
const ENTRY_SIZE: usize = 64;
const MAX_FILES: usize = DIRECTORY_BLOCKS as usize * BLOCK_SIZE / ENTRY_SIZE;
const MAX_NAME_LEN: usize = 32;
const MAX_FILE_SIZE: usize = BLOCK_SIZE * 2;
const BLOCKS_PER_FILE: u64 = 2;
const CACHE_ENTRIES: usize = 16;

type PersistentPartition = PartitionBlockDevice<AtaPioDisk>;
type PersistentDisk = CachedBlockDevice<PersistentPartition, CACHE_ENTRIES>;

static DISK: PreemptMutex<PersistentDisk> =
    PreemptMutex::new(PersistentDisk::new(PersistentPartition::new(
        AtaPioDisk::primary_slave(ATA_ADDRESSABLE_BLOCKS),
        ATA_ADDRESSABLE_BLOCKS,
        VOLUME_BLOCKS,
    )));

pub struct PersistentFilesystem {
    available: AtomicBool,
}

impl PersistentFilesystem {
    pub const fn new() -> Self {
        Self {
            available: AtomicBool::new(false),
        }
    }

    pub fn init(&self) {
        let result = self.initialize_disk();
        match result {
            Ok(formatted) => {
                self.available.store(true, Ordering::Release);
                let detail = if formatted {
                    "formatted new VANTFS01 volume"
                } else {
                    "mounted existing VANTFS01 volume"
                };
                crate::drivers::status::report(
                    "ata-persist",
                    crate::drivers::status::DriverState::Ready,
                    detail,
                );
                crate::drivers::status::report(
                    "block-cache",
                    crate::drivers::status::DriverState::Ready,
                    "16-sector LRU write-through",
                );
                crate::serial_println!("[PERSIST] {}", detail);
                crate::serial_println!(
                    "[BLOCKCACHE] ready capacity={} policy=write-through",
                    CACHE_ENTRIES
                );
            }
            Err(error) => {
                self.available.store(false, Ordering::Release);
                crate::drivers::status::report(
                    "ata-persist",
                    crate::drivers::status::DriverState::Missing,
                    "dedicated IDE persistence disk unavailable",
                );
                crate::serial_println!("[PERSIST] unavailable: {:?}", error);
            }
        }
    }

    pub fn available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }

    fn initialize_disk(&self) -> Result<bool, FsError> {
        let mut disk = DISK.lock();
        let partition = disk.backing_mut();
        partition
            .device()
            .probe()
            .map_err(|_| FsError::Unavailable)?;
        let mut mbr = [0u8; BLOCK_SIZE];
        partition
            .device_mut()
            .read_block(0, &mut mbr)
            .map_err(|_| FsError::Io)?;
        match parse_mbr(&mbr, ATA_ADDRESSABLE_BLOCKS) {
            Ok(partitions) => {
                let selected = select_vantara_partition(&partitions).ok_or(FsError::Unavailable)?;
                if selected.block_count < VOLUME_BLOCKS {
                    return Err(FsError::Unavailable);
                }
                partition
                    .configure_partition(selected)
                    .map_err(|_| FsError::Unavailable)?;
                crate::serial_println!(
                    "[PARTITION] MBR type={:#04x} start={} blocks={}",
                    selected.partition_type,
                    selected.start_block,
                    selected.block_count
                );
            }
            Err(PartitionError::MissingSignature) => {
                partition
                    .configure_superfloppy(VOLUME_BLOCKS)
                    .map_err(|_| FsError::Unavailable)?;
                crate::serial_println!(
                    "[PARTITION] no MBR; superfloppy start=0 blocks={}",
                    VOLUME_BLOCKS
                );
            }
            Err(error) => {
                crate::serial_println!("[PARTITION] invalid MBR: {:?}", error);
                return Err(FsError::Corrupt);
            }
        }
        let mut header = [0u8; BLOCK_SIZE];
        disk.read_block(HEADER_BLOCK, &mut header)
            .map_err(|_| FsError::Io)?;
        if &header[..MAGIC.len()] == MAGIC {
            if read_u32(&header, 8) != VERSION {
                return Err(FsError::Corrupt);
            }
            return Ok(false);
        }
        format_volume(&mut *disk)?;
        Ok(true)
    }

    fn ensure_available(&self) -> Result<(), FsError> {
        self.available().then_some(()).ok_or(FsError::Unavailable)
    }
}

pub(super) fn write_partition_info_to_buffer(out: &mut [u8]) -> usize {
    let disk = DISK.lock();
    let partition = disk.backing();
    let mut writer = CacheStatsWriter::new(out);
    writer.write_str("VANTFS storage view\nmode ");
    writer.write_str(if partition.partitioned() {
        "mbr"
    } else {
        "superfloppy"
    });
    writer.write_str("\ntype ");
    writer.write_hex_u8(partition.partition_type());
    writer.write_str("\nstart_block ");
    writer.write_dec(partition.start_block());
    writer.write_str("\nblock_count ");
    writer.write_dec(partition.block_count());
    writer.write_byte(b'\n');
    writer.len()
}

pub(super) fn write_cache_stats_to_buffer(out: &mut [u8]) -> usize {
    let disk = DISK.lock();
    let stats = disk.stats();
    let mut writer = CacheStatsWriter::new(out);
    writer.write_str("VANTFS block cache\ncapacity ");
    writer.write_dec(disk.capacity() as u64);
    writer.write_str("\nread_hits ");
    writer.write_dec(stats.read_hits);
    writer.write_str("\nread_misses ");
    writer.write_dec(stats.read_misses);
    writer.write_str("\nwrites ");
    writer.write_dec(stats.writes);
    writer.write_str("\nevictions ");
    writer.write_dec(stats.evictions);
    writer.write_byte(b'\n');
    writer.len()
}

struct CacheStatsWriter<'a> {
    out: &'a mut [u8],
    len: usize,
}

impl<'a> CacheStatsWriter<'a> {
    fn new(out: &'a mut [u8]) -> Self {
        Self { out, len: 0 }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn write_byte(&mut self, byte: u8) {
        if self.len < self.out.len() {
            self.out[self.len] = byte;
            self.len += 1;
        }
    }

    fn write_str(&mut self, text: &str) {
        for byte in text.bytes() {
            self.write_byte(byte);
        }
    }

    fn write_dec(&mut self, mut value: u64) {
        let mut digits = [0u8; 20];
        let mut len = 0;
        if value == 0 {
            self.write_byte(b'0');
            return;
        }
        while value > 0 {
            digits[len] = b'0' + (value % 10) as u8;
            value /= 10;
            len += 1;
        }
        while len > 0 {
            len -= 1;
            self.write_byte(digits[len]);
        }
    }

    fn write_hex_u8(&mut self, value: u8) {
        self.write_str("0x");
        for shift in [4, 0] {
            let nibble = (value >> shift) & 0x0f;
            self.write_byte(if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            });
        }
    }
}

impl FilesystemBackend for PersistentFilesystem {
    fn name(&self) -> &'static str {
        "vantfs"
    }

    fn available(&self) -> bool {
        self.available()
    }

    fn list(&self, path: &str, out: &mut [u8]) -> Result<usize, FsError> {
        self.ensure_available()?;
        let mut disk = DISK.lock();
        let entries = read_entries(&mut *disk)?;
        let parent = resolve_path(&entries, path).ok_or(FsError::NotFound)?;
        if parent != 1
            && entries
                .iter()
                .flatten()
                .find(|entry| entry.inode == parent)
                .is_none_or(|entry| entry.file_type != FileType::Directory)
        {
            return Err(FsError::NotDirectory);
        }
        let mut written = 0;
        for entry in entries
            .iter()
            .flatten()
            .filter(|entry| entry.parent == parent)
        {
            written = append_line(out, written, entry.name().as_bytes())?;
        }
        Ok(written)
    }

    fn stat(&self, path: &str) -> Result<BackendStat, FsError> {
        self.ensure_available()?;
        let mut disk = DISK.lock();
        let entries = read_entries(&mut *disk)?;
        if path == "/" {
            let size = entries
                .iter()
                .flatten()
                .filter(|entry| entry.parent == 1)
                .count();
            return Ok(BackendStat {
                file_type: FileType::Directory,
                size,
                readonly: false,
                inode: 1,
            });
        }
        let inode = resolve_path(&entries, path).ok_or(FsError::NotFound)?;
        let entry = entries
            .iter()
            .flatten()
            .find(|entry| entry.inode == inode)
            .ok_or(FsError::NotFound)?;
        let mut stat = entry.stat();
        if entry.file_type == FileType::Directory {
            stat.size = entries
                .iter()
                .flatten()
                .filter(|child| child.parent == entry.inode)
                .count();
        }
        Ok(stat)
    }

    fn read(&self, inode: u64, offset: usize, out: &mut [u8]) -> Result<usize, FsError> {
        self.ensure_available()?;
        let mut disk = DISK.lock();
        let entries = read_entries(&mut *disk)?;
        let entry = entries
            .iter()
            .flatten()
            .find(|entry| entry.inode == inode)
            .ok_or(FsError::NotFound)?;
        if entry.file_type != FileType::File {
            return Err(FsError::IsDirectory);
        }
        let count = entry.len.saturating_sub(offset).min(out.len());
        read_data(&mut *disk, entry.slot, offset, &mut out[..count])?;
        Ok(count)
    }

    fn write(&self, inode: u64, offset: usize, source: &[u8]) -> Result<usize, FsError> {
        self.ensure_available()?;
        let end = offset
            .checked_add(source.len())
            .ok_or(FsError::FileTooLarge)?;
        if end > MAX_FILE_SIZE {
            return Err(FsError::FileTooLarge);
        }
        let mut disk = DISK.lock();
        let mut entries = read_entries(&mut *disk)?;
        let entry = entries
            .iter_mut()
            .flatten()
            .find(|entry| entry.inode == inode)
            .ok_or(FsError::NotFound)?;
        if entry.file_type != FileType::File {
            return Err(FsError::IsDirectory);
        }
        write_data(&mut *disk, entry.slot, offset, source)?;
        entry.len = entry.len.max(end);
        write_entry(&mut *disk, *entry)?;
        Ok(source.len())
    }

    fn create(&self, path: &str) -> Result<BackendStat, FsError> {
        self.create_node(path, FileType::File)
    }

    fn mkdir(&self, path: &str) -> Result<BackendStat, FsError> {
        self.create_node(path, FileType::Directory)
    }

    fn unlink(&self, path: &str) -> Result<u64, FsError> {
        self.remove_node(path, FileType::File)
    }

    fn rmdir(&self, path: &str) -> Result<u64, FsError> {
        self.remove_node(path, FileType::Directory)
    }

    fn rename(&self, old_path: &str, new_path: &str) -> Result<u64, FsError> {
        self.ensure_available()?;
        let mut disk = DISK.lock();
        let entries = read_entries(&mut *disk)?;
        let inode = resolve_path(&entries, old_path).ok_or(FsError::NotFound)?;
        if inode == 1 {
            return Err(FsError::ReadOnly);
        }
        let (parent_path, new_name) = split_parent_name(new_path)?;
        let new_parent = resolve_path(&entries, parent_path).ok_or(FsError::NotFound)?;
        if new_parent != 1
            && entries
                .iter()
                .flatten()
                .find(|entry| entry.inode == new_parent)
                .is_none_or(|entry| entry.file_type != FileType::Directory)
        {
            return Err(FsError::NotDirectory);
        }
        if child(&entries, new_parent, new_name).is_some() {
            return Err(FsError::AlreadyExists);
        }
        if is_descendant(&entries, new_parent, inode) {
            return Err(FsError::InvalidPath);
        }
        let mut entry = entries
            .iter()
            .flatten()
            .find(|entry| entry.inode == inode)
            .copied()
            .ok_or(FsError::NotFound)?;
        entry.parent = new_parent;
        entry.name.fill(0);
        entry.name[..new_name.len()].copy_from_slice(new_name);
        entry.name_len = new_name.len();
        write_entry(&mut *disk, entry)?;
        Ok(entry.inode)
    }
}

impl PersistentFilesystem {
    fn create_node(&self, path: &str, file_type: FileType) -> Result<BackendStat, FsError> {
        self.ensure_available()?;
        let (parent_path, name) = split_parent_name(path)?;
        let mut disk = DISK.lock();
        let entries = read_entries(&mut *disk)?;
        let parent = resolve_path(&entries, parent_path).ok_or(FsError::NotFound)?;
        if parent != 1
            && entries
                .iter()
                .flatten()
                .find(|entry| entry.inode == parent)
                .is_none_or(|entry| entry.file_type != FileType::Directory)
        {
            return Err(FsError::NotDirectory);
        }
        if child(&entries, parent, name).is_some() {
            return Err(FsError::AlreadyExists);
        }
        let slot = entries
            .iter()
            .position(Option::is_none)
            .ok_or(FsError::NoSpace)?;
        if file_type == FileType::File {
            clear_data(&mut *disk, slot)?;
        }
        let entry = DirectoryEntry::new(slot, parent, name, file_type);
        write_entry(&mut *disk, entry)?;
        Ok(entry.stat())
    }

    fn remove_node(&self, path: &str, expected: FileType) -> Result<u64, FsError> {
        self.ensure_available()?;
        let mut disk = DISK.lock();
        let entries = read_entries(&mut *disk)?;
        let inode = resolve_path(&entries, path).ok_or(FsError::NotFound)?;
        if inode == 1 {
            return Err(FsError::ReadOnly);
        }
        let entry = entries
            .iter()
            .flatten()
            .find(|entry| entry.inode == inode)
            .copied()
            .ok_or(FsError::NotFound)?;
        if entry.file_type != expected {
            return if expected == FileType::File {
                Err(FsError::IsDirectory)
            } else {
                Err(FsError::NotDirectory)
            };
        }
        if expected == FileType::Directory
            && entries.iter().flatten().any(|child| child.parent == inode)
        {
            return Err(FsError::DirectoryNotEmpty);
        }
        write_empty_entry(&mut *disk, entry.slot)?;
        if expected == FileType::File {
            clear_data(&mut *disk, entry.slot)?;
        }
        Ok(entry.inode)
    }
}

#[derive(Clone, Copy)]
struct DirectoryEntry {
    slot: usize,
    inode: u64,
    len: usize,
    parent: u64,
    file_type: FileType,
    name: [u8; MAX_NAME_LEN],
    name_len: usize,
}

impl DirectoryEntry {
    fn new(slot: usize, parent: u64, name: &[u8], file_type: FileType) -> Self {
        let mut entry = Self {
            slot,
            inode: slot as u64 + 2,
            len: 0,
            parent,
            file_type,
            name: [0; MAX_NAME_LEN],
            name_len: name.len(),
        };
        entry.name[..name.len()].copy_from_slice(name);
        entry
    }

    fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }

    fn stat(&self) -> BackendStat {
        BackendStat {
            file_type: self.file_type,
            size: self.len,
            readonly: false,
            inode: self.inode,
        }
    }
}

fn format_volume(disk: &mut impl BlockDevice) -> Result<(), FsError> {
    let mut block = [0u8; BLOCK_SIZE];
    block[..MAGIC.len()].copy_from_slice(MAGIC);
    block[8..12].copy_from_slice(&VERSION.to_le_bytes());
    block[12..16].copy_from_slice(&(MAX_FILES as u32).to_le_bytes());
    disk.write_block(HEADER_BLOCK, &block)
        .map_err(|_| FsError::Io)?;
    block.fill(0);
    for index in 0..DIRECTORY_BLOCKS {
        disk.write_block(DIRECTORY_START_BLOCK + index, &block)
            .map_err(|_| FsError::Io)?;
    }
    Ok(())
}

fn read_entries(disk: &mut impl BlockDevice) -> Result<Vec<Option<DirectoryEntry>>, FsError> {
    let mut entries = Vec::with_capacity(MAX_FILES);
    entries.resize(MAX_FILES, None);
    let mut block = [0u8; BLOCK_SIZE];
    for block_index in 0..DIRECTORY_BLOCKS {
        disk.read_block(DIRECTORY_START_BLOCK + block_index, &mut block)
            .map_err(|_| FsError::Io)?;
        for local in 0..BLOCK_SIZE / ENTRY_SIZE {
            let slot = block_index as usize * (BLOCK_SIZE / ENTRY_SIZE) + local;
            let start = local * ENTRY_SIZE;
            if block[start] == 0 {
                continue;
            }
            let name_len = block[start + 1] as usize;
            if name_len == 0 || name_len > MAX_NAME_LEN {
                return Err(FsError::Corrupt);
            }
            let mut name = [0u8; MAX_NAME_LEN];
            name.copy_from_slice(&block[start + 24..start + 24 + MAX_NAME_LEN]);
            entries[slot] = Some(DirectoryEntry {
                slot,
                inode: read_u64(&block, start + 8),
                len: read_u32(&block, start + 16) as usize,
                parent: match read_u32(&block, start + 20) {
                    0 => 1,
                    parent => parent as u64,
                },
                file_type: match block[start + 2] {
                    0 | 1 => FileType::File,
                    2 => FileType::Directory,
                    _ => return Err(FsError::Corrupt),
                },
                name,
                name_len,
            });
        }
    }
    Ok(entries)
}

fn write_entry(disk: &mut impl BlockDevice, entry: DirectoryEntry) -> Result<(), FsError> {
    let block_index = entry.slot / (BLOCK_SIZE / ENTRY_SIZE);
    let local = entry.slot % (BLOCK_SIZE / ENTRY_SIZE);
    let mut block = [0u8; BLOCK_SIZE];
    disk.read_block(DIRECTORY_START_BLOCK + block_index as u64, &mut block)
        .map_err(|_| FsError::Io)?;
    let start = local * ENTRY_SIZE;
    block[start..start + ENTRY_SIZE].fill(0);
    block[start] = 1;
    block[start + 1] = entry.name_len as u8;
    block[start + 2] = match entry.file_type {
        FileType::File => 1,
        FileType::Directory => 2,
    };
    block[start + 8..start + 16].copy_from_slice(&entry.inode.to_le_bytes());
    block[start + 16..start + 20].copy_from_slice(&(entry.len as u32).to_le_bytes());
    block[start + 20..start + 24].copy_from_slice(&(entry.parent as u32).to_le_bytes());
    block[start + 24..start + 24 + MAX_NAME_LEN].copy_from_slice(&entry.name);
    disk.write_block(DIRECTORY_START_BLOCK + block_index as u64, &block)
        .map_err(|_| FsError::Io)
}

fn write_empty_entry(disk: &mut impl BlockDevice, slot: usize) -> Result<(), FsError> {
    let block_index = slot / (BLOCK_SIZE / ENTRY_SIZE);
    let local = slot % (BLOCK_SIZE / ENTRY_SIZE);
    let mut block = [0u8; BLOCK_SIZE];
    disk.read_block(DIRECTORY_START_BLOCK + block_index as u64, &mut block)
        .map_err(|_| FsError::Io)?;
    let start = local * ENTRY_SIZE;
    block[start..start + ENTRY_SIZE].fill(0);
    disk.write_block(DIRECTORY_START_BLOCK + block_index as u64, &block)
        .map_err(|_| FsError::Io)
}

fn read_data(
    disk: &mut impl BlockDevice,
    slot: usize,
    offset: usize,
    out: &mut [u8],
) -> Result<(), FsError> {
    transfer_data(slot, offset, out, |block_index, block| {
        disk.read_block(block_index, block).map_err(|_| FsError::Io)
    })
}

fn write_data(
    disk: &mut impl BlockDevice,
    slot: usize,
    offset: usize,
    source: &[u8],
) -> Result<(), FsError> {
    let mut copied = 0;
    while copied < source.len() {
        let absolute = offset + copied;
        let file_block = absolute / BLOCK_SIZE;
        let within = absolute % BLOCK_SIZE;
        let count = (BLOCK_SIZE - within).min(source.len() - copied);
        let block_index = data_block(slot, file_block);
        let mut block = [0u8; BLOCK_SIZE];
        disk.read_block(block_index, &mut block)
            .map_err(|_| FsError::Io)?;
        block[within..within + count].copy_from_slice(&source[copied..copied + count]);
        disk.write_block(block_index, &block)
            .map_err(|_| FsError::Io)?;
        copied += count;
    }
    Ok(())
}

fn transfer_data<F>(slot: usize, offset: usize, out: &mut [u8], mut read: F) -> Result<(), FsError>
where
    F: FnMut(u64, &mut [u8; BLOCK_SIZE]) -> Result<(), FsError>,
{
    let mut copied = 0;
    while copied < out.len() {
        let absolute = offset + copied;
        let file_block = absolute / BLOCK_SIZE;
        let within = absolute % BLOCK_SIZE;
        let count = (BLOCK_SIZE - within).min(out.len() - copied);
        let mut block = [0u8; BLOCK_SIZE];
        read(data_block(slot, file_block), &mut block)?;
        out[copied..copied + count].copy_from_slice(&block[within..within + count]);
        copied += count;
    }
    Ok(())
}

fn clear_data(disk: &mut impl BlockDevice, slot: usize) -> Result<(), FsError> {
    let block = [0u8; BLOCK_SIZE];
    for file_block in 0..BLOCKS_PER_FILE {
        disk.write_block(data_block(slot, file_block as usize), &block)
            .map_err(|_| FsError::Io)?;
    }
    Ok(())
}

fn data_block(slot: usize, file_block: usize) -> u64 {
    DATA_START_BLOCK + slot as u64 * BLOCKS_PER_FILE + file_block as u64
}

fn split_parent_name(path: &str) -> Result<(&str, &[u8]), FsError> {
    if !path.starts_with('/') || path == "/" {
        return Err(FsError::InvalidPath);
    }
    let split = path.rfind('/').ok_or(FsError::InvalidPath)?;
    let parent = if split == 0 { "/" } else { &path[..split] };
    let name = path[split + 1..].as_bytes();
    if name.is_empty() {
        return Err(FsError::InvalidPath);
    }
    if name.len() > MAX_NAME_LEN {
        return Err(FsError::NameTooLong);
    }
    Ok((parent, name))
}

fn child<'a>(
    entries: &'a [Option<DirectoryEntry>],
    parent: u64,
    name: &[u8],
) -> Option<&'a DirectoryEntry> {
    entries
        .iter()
        .flatten()
        .find(|entry| entry.parent == parent && entry.name().as_bytes() == name)
}

fn resolve_path(entries: &[Option<DirectoryEntry>], path: &str) -> Option<u64> {
    if path == "/" {
        return Some(1);
    }
    let mut parent = 1;
    for component in path.strip_prefix('/')?.split('/') {
        if component.is_empty() {
            return None;
        }
        parent = child(entries, parent, component.as_bytes())?.inode;
    }
    Some(parent)
}

fn is_descendant(entries: &[Option<DirectoryEntry>], mut candidate: u64, ancestor: u64) -> bool {
    while candidate != 1 {
        if candidate == ancestor {
            return true;
        }
        let Some(entry) = entries
            .iter()
            .flatten()
            .find(|entry| entry.inode == candidate)
        else {
            return false;
        };
        candidate = entry.parent;
    }
    false
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap_or([0; 4]))
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap_or([0; 8]))
}

#[cfg(test)]
mod tests {
    use super::{DirectoryEntry, MAX_FILES, data_block, resolve_path, split_parent_name};
    use crate::fs::FsError;

    #[test_case]
    fn persistent_layout_allocates_two_blocks_per_file() {
        assert_eq!(data_block(0, 0), 5);
        assert_eq!(data_block(0, 1), 6);
        assert_eq!(data_block(1, 0), 7);
        assert_eq!(MAX_FILES, 32);
    }

    #[test_case]
    fn persistent_paths_split_parent_and_name() {
        assert_eq!(split_parent_name("/note"), Ok(("/", &b"note"[..])));
        assert_eq!(
            split_parent_name("/nested/note"),
            Ok(("/nested", &b"note"[..]))
        );
        assert_eq!(split_parent_name("/"), Err(FsError::InvalidPath));
    }

    #[test_case]
    fn persistent_tree_resolves_nested_nodes() {
        let mut entries = alloc::vec![None; MAX_FILES];
        entries[0] = Some(DirectoryEntry::new(
            0,
            1,
            b"docs",
            super::FileType::Directory,
        ));
        entries[1] = Some(DirectoryEntry::new(1, 2, b"note", super::FileType::File));
        assert_eq!(resolve_path(&entries, "/docs"), Some(2));
        assert_eq!(resolve_path(&entries, "/docs/note"), Some(3));
    }
}
