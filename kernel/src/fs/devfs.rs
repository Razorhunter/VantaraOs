use super::{BackendStat, FileType, FilesystemBackend, FsError, append_line};

const ROOT_INODE: u64 = 1;
const NULL_INODE: u64 = 2;
const ZERO_INODE: u64 = 3;
const DRIVERS_INODE: u64 = 4;
const PCI_INODE: u64 = 5;
const NET_INODE: u64 = 6;
const BLOCK_CACHE_INODE: u64 = 7;
const PARTITIONS_INODE: u64 = 8;
const AHCI_INODE: u64 = 9;
const SNAPSHOT_SIZE: usize = 2048;

pub struct DeviceFilesystem;

impl DeviceFilesystem {
    pub const fn new() -> Self {
        Self
    }

    pub fn init(&self) {
        crate::drivers::status::report(
            "devfs",
            crate::drivers::status::DriverState::Ready,
            "mounted at /dev",
        );
        crate::serial_println!(
            "[DEVFS] ready: nodes=null,zero,drivers,pci,net,block-cache,partitions,ahci"
        );
    }
}

impl FilesystemBackend for DeviceFilesystem {
    fn name(&self) -> &'static str {
        "devfs"
    }

    fn list(&self, path: &str, out: &mut [u8]) -> Result<usize, FsError> {
        if path != "/" {
            return Err(FsError::NotDirectory);
        }
        let mut written = 0;
        for name in [
            "null",
            "zero",
            "drivers",
            "pci",
            "net",
            "block-cache",
            "partitions",
            "ahci",
        ] {
            written = append_line(out, written, name.as_bytes())?;
        }
        Ok(written)
    }

    fn stat(&self, path: &str) -> Result<BackendStat, FsError> {
        let (file_type, size, readonly, inode) = match path {
            "/" => (FileType::Directory, 8, true, ROOT_INODE),
            "/null" => (FileType::File, 0, false, NULL_INODE),
            "/zero" => (FileType::File, 0, false, ZERO_INODE),
            "/drivers" => (
                FileType::File,
                snapshot_len(crate::drivers::status::write_to_buffer),
                true,
                DRIVERS_INODE,
            ),
            "/pci" => (
                FileType::File,
                snapshot_len(crate::drivers::pci::write_devices_to_buffer),
                true,
                PCI_INODE,
            ),
            "/net" => (
                FileType::File,
                snapshot_len(crate::drivers::network::write_devices_to_buffer),
                true,
                NET_INODE,
            ),
            "/block-cache" => (
                FileType::File,
                snapshot_len(super::persistent::write_cache_stats_to_buffer),
                true,
                BLOCK_CACHE_INODE,
            ),
            "/partitions" => (
                FileType::File,
                snapshot_len(super::persistent::write_partition_info_to_buffer),
                true,
                PARTITIONS_INODE,
            ),
            "/ahci" => (
                FileType::File,
                snapshot_len(crate::drivers::ahci::write_to_buffer),
                true,
                AHCI_INODE,
            ),
            _ => return Err(FsError::NotFound),
        };
        Ok(BackendStat {
            file_type,
            size,
            readonly,
            inode,
        })
    }

    fn read(&self, inode: u64, offset: usize, out: &mut [u8]) -> Result<usize, FsError> {
        match inode {
            NULL_INODE => Ok(0),
            ZERO_INODE => {
                out.fill(0);
                Ok(out.len())
            }
            DRIVERS_INODE => read_snapshot(offset, out, crate::drivers::status::write_to_buffer),
            PCI_INODE => read_snapshot(offset, out, crate::drivers::pci::write_devices_to_buffer),
            NET_INODE => read_snapshot(
                offset,
                out,
                crate::drivers::network::write_devices_to_buffer,
            ),
            BLOCK_CACHE_INODE => {
                read_snapshot(offset, out, super::persistent::write_cache_stats_to_buffer)
            }
            PARTITIONS_INODE => read_snapshot(
                offset,
                out,
                super::persistent::write_partition_info_to_buffer,
            ),
            AHCI_INODE => read_snapshot(offset, out, crate::drivers::ahci::write_to_buffer),
            ROOT_INODE => Err(FsError::IsDirectory),
            _ => Err(FsError::NotFound),
        }
    }

    fn write(&self, inode: u64, _offset: usize, source: &[u8]) -> Result<usize, FsError> {
        match inode {
            NULL_INODE | ZERO_INODE => Ok(source.len()),
            DRIVERS_INODE | PCI_INODE | NET_INODE | BLOCK_CACHE_INODE | PARTITIONS_INODE
            | AHCI_INODE => Err(FsError::ReadOnly),
            ROOT_INODE => Err(FsError::IsDirectory),
            _ => Err(FsError::NotFound),
        }
    }

    fn create(&self, _path: &str) -> Result<BackendStat, FsError> {
        Err(FsError::ReadOnly)
    }

    fn mkdir(&self, _path: &str) -> Result<BackendStat, FsError> {
        Err(FsError::ReadOnly)
    }

    fn unlink(&self, _path: &str) -> Result<u64, FsError> {
        Err(FsError::ReadOnly)
    }

    fn rmdir(&self, _path: &str) -> Result<u64, FsError> {
        Err(FsError::ReadOnly)
    }

    fn rename(&self, _old_path: &str, _new_path: &str) -> Result<u64, FsError> {
        Err(FsError::ReadOnly)
    }
}

fn snapshot_len(writer: fn(&mut [u8]) -> usize) -> usize {
    let mut snapshot = [0u8; SNAPSHOT_SIZE];
    writer(&mut snapshot)
}

fn read_snapshot(
    offset: usize,
    out: &mut [u8],
    writer: fn(&mut [u8]) -> usize,
) -> Result<usize, FsError> {
    let mut snapshot = [0u8; SNAPSHOT_SIZE];
    let len = writer(&mut snapshot);
    let remaining = len.saturating_sub(offset);
    let count = remaining.min(out.len());
    out[..count].copy_from_slice(&snapshot[offset..offset + count]);
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::{DeviceFilesystem, NULL_INODE, ZERO_INODE};
    use crate::fs::FilesystemBackend;

    #[test_case]
    fn lists_stable_device_nodes() {
        let devfs = DeviceFilesystem::new();
        let mut out = [0u8; 64];
        let len = devfs.list("/", &mut out).unwrap();
        assert_eq!(
            &out[..len],
            b"null\nzero\ndrivers\npci\nnet\nblock-cache\npartitions\nahci\n"
        );
    }

    #[test_case]
    fn null_discards_writes_and_returns_eof() {
        let devfs = DeviceFilesystem::new();
        let mut out = [0u8; 8];
        assert_eq!(devfs.write(NULL_INODE, 0, b"discard").unwrap(), 7);
        assert_eq!(devfs.read(NULL_INODE, 0, &mut out).unwrap(), 0);
    }

    #[test_case]
    fn zero_returns_zero_filled_bytes() {
        let devfs = DeviceFilesystem::new();
        let mut out = [0xffu8; 8];
        assert_eq!(devfs.read(ZERO_INODE, 0, &mut out).unwrap(), 8);
        assert_eq!(out, [0; 8]);
    }
}
