use alloc::vec::Vec;
use lazy_static::lazy_static;

use super::{BackendStat, FileType, FilesystemBackend, FsError, append_line};
use crate::storage::ramdisk::RamDisk;
use crate::sync::PreemptMutex;

const MAX_WRITABLE_NODES: usize = 32;
const MAX_FILE_NAME_LEN: usize = 32;
const MAX_FILE_SIZE: usize = 1024;
const ROOT_INODE: u64 = 1;
const BIN_INODE: u64 = 2;
const TMP_INODE: u64 = 3;
const FIRST_WRITABLE_INODE: u64 = 10_000;
const BASE_DIRECTORY_INODE: u64 = 20_000;
const BASE_DIRECTORIES: &[&str] = &[
    "System", "Apps", "Users", "Config", "Data", "Cache", "Logs", "Runtime", "Devices", "Temp",
    "Packages", "Boot", "Volumes",
];

static README: &[u8] =
    b"Vantara OS RAM filesystem\n\nThis filesystem has read-only system files and writable /tmp storage.\n";
static VERSION: &[u8] = env!("CARGO_PKG_VERSION").as_bytes();
static MOTD: &[u8] =
    b"Welcome to Vantara OS.\nTry: help, uptime, mem, tasks, pci, ls, cat, stat.\n";
static RAMDISK_BYTES: &[u8] = b"vantara-ramdisk";
static RAMDISK: RamDisk = RamDisk::new(RAMDISK_BYTES);

lazy_static! {
    static ref WRITABLE_FS: PreemptMutex<WritableStore> = PreemptMutex::new(WritableStore::new());
}

pub struct RamFilesystem;

impl RamFilesystem {
    pub const fn new() -> Self {
        Self
    }

    pub fn init(&self) {
        crate::serial_println!(
            "[RAMFS] ready: readonly_files={} writable_capacity={} ramdisk_bytes={} blocks={}",
            FILES.len() + crate::user::images::USERLAND_IMAGES.len(),
            MAX_WRITABLE_NODES,
            RAMDISK.len(),
            crate::storage::block::BlockDevice::block_count(&RAMDISK)
        );
    }
}

impl FilesystemBackend for RamFilesystem {
    fn name(&self) -> &'static str {
        "ramfs"
    }

    fn list(&self, path: &str, out: &mut [u8]) -> Result<usize, FsError> {
        let mut written = 0;
        match path {
            "/" => {
                for name in ["README", "VERSION", "MOTD", "bin", "tmp"]
                    .into_iter()
                    .chain(BASE_DIRECTORIES.iter().copied())
                {
                    written = append_line(out, written, name.as_bytes())?;
                }
            }
            "/bin" | "/Apps" => {
                for name in crate::user::images::USERLAND_IMAGE_NAMES {
                    written = append_line(out, written, name.as_bytes())?;
                }
            }
            path if path != "/Temp" && base_directory(path).is_some() => {}
            path => {
                let store = WRITABLE_FS.lock();
                let directory = store.resolve_path(path).ok_or(FsError::NotFound)?;
                if directory != TMP_INODE
                    && store
                        .find_inode(directory)
                        .is_none_or(|node| node.file_type != FileType::Directory)
                {
                    return Err(FsError::NotDirectory);
                }
                for node in store.nodes.iter().filter(|node| node.parent == directory) {
                    written = append_line(out, written, node.name().as_bytes())?;
                }
            }
        }
        Ok(written)
    }

    fn stat(&self, path: &str) -> Result<BackendStat, FsError> {
        let (file_type, size, readonly, inode) = match path {
            "/" => (
                FileType::Directory,
                5 + BASE_DIRECTORIES.len(),
                true,
                ROOT_INODE,
            ),
            "/bin" | "/Apps" => (
                FileType::Directory,
                crate::user::images::USERLAND_IMAGE_NAMES.len(),
                true,
                BIN_INODE,
            ),
            "/tmp" | "/Temp" => {
                let store = WRITABLE_FS.lock();
                (
                    FileType::Directory,
                    store
                        .nodes
                        .iter()
                        .filter(|node| node.parent == TMP_INODE)
                        .count(),
                    false,
                    TMP_INODE,
                )
            }
            path if base_directory(path).is_some() => {
                let (inode, _) = base_directory(path).unwrap_or((0, ""));
                (FileType::Directory, 0, true, inode)
            }
            path => {
                if let Some(entry) = find_static(path) {
                    (FileType::File, entry.data().len(), true, entry.inode())
                } else {
                    let store = WRITABLE_FS.lock();
                    let inode = store.resolve_path(path).ok_or(FsError::NotFound)?;
                    let node = store.find_inode(inode).ok_or(FsError::NotFound)?;
                    let size = if node.file_type == FileType::Directory {
                        store
                            .nodes
                            .iter()
                            .filter(|child| child.parent == node.inode)
                            .count()
                    } else {
                        node.len
                    };
                    (node.file_type, size, false, node.inode)
                }
            }
        };
        Ok(BackendStat {
            file_type,
            size,
            readonly,
            inode,
        })
    }

    fn read(&self, inode: u64, offset: usize, out: &mut [u8]) -> Result<usize, FsError> {
        if let Some(entry) = static_by_inode(inode) {
            let remaining = entry.data().len().saturating_sub(offset);
            let count = remaining.min(out.len());
            out[..count].copy_from_slice(&entry.data()[offset..offset + count]);
            return Ok(count);
        }
        let store = WRITABLE_FS.lock();
        let node = store.find_inode(inode).ok_or(FsError::NotFound)?;
        if node.file_type != FileType::File {
            return Err(FsError::IsDirectory);
        }
        let remaining = node.len.saturating_sub(offset);
        let count = remaining.min(out.len());
        out[..count].copy_from_slice(&node.data[offset..offset + count]);
        Ok(count)
    }

    fn write(&self, inode: u64, offset: usize, source: &[u8]) -> Result<usize, FsError> {
        WRITABLE_FS.lock().write(inode, offset, source)
    }

    fn create(&self, path: &str) -> Result<BackendStat, FsError> {
        let inode = WRITABLE_FS.lock().create(path, FileType::File)?;
        Ok(BackendStat {
            file_type: FileType::File,
            size: 0,
            readonly: false,
            inode,
        })
    }

    fn mkdir(&self, path: &str) -> Result<BackendStat, FsError> {
        let inode = WRITABLE_FS.lock().create(path, FileType::Directory)?;
        Ok(BackendStat {
            file_type: FileType::Directory,
            size: 0,
            readonly: false,
            inode,
        })
    }

    fn unlink(&self, path: &str) -> Result<u64, FsError> {
        WRITABLE_FS.lock().remove(path, FileType::File)
    }

    fn rmdir(&self, path: &str) -> Result<u64, FsError> {
        WRITABLE_FS.lock().remove(path, FileType::Directory)
    }

    fn rename(&self, old_path: &str, new_path: &str) -> Result<u64, FsError> {
        WRITABLE_FS.lock().rename(old_path, new_path)
    }
}

#[derive(Clone, Copy)]
struct FileEntry {
    inode: u64,
    path: &'static str,
    data: &'static [u8],
}

const FILES: &[FileEntry] = &[
    FileEntry {
        inode: 10,
        path: "/README",
        data: README,
    },
    FileEntry {
        inode: 11,
        path: "/VERSION",
        data: VERSION,
    },
    FileEntry {
        inode: 12,
        path: "/MOTD",
        data: MOTD,
    },
];

#[derive(Clone, Copy)]
struct WritableNode {
    inode: u64,
    parent: u64,
    name: [u8; MAX_FILE_NAME_LEN],
    name_len: usize,
    file_type: FileType,
    data: [u8; MAX_FILE_SIZE],
    len: usize,
}

impl WritableNode {
    fn new(inode: u64, parent: u64, name: &[u8], file_type: FileType) -> Self {
        let mut node = Self {
            inode,
            parent,
            name: [0; MAX_FILE_NAME_LEN],
            name_len: name.len(),
            file_type,
            data: [0; MAX_FILE_SIZE],
            len: 0,
        };
        node.name[..name.len()].copy_from_slice(name);
        node
    }

    fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }
}

struct WritableStore {
    nodes: Vec<WritableNode>,
    next_inode: u64,
}

impl WritableStore {
    fn new() -> Self {
        Self {
            nodes: Vec::with_capacity(MAX_WRITABLE_NODES),
            next_inode: FIRST_WRITABLE_INODE,
        }
    }

    fn child(&self, parent: u64, name: &[u8]) -> Option<&WritableNode> {
        self.nodes
            .iter()
            .find(|node| node.parent == parent && node.name().as_bytes() == name)
    }

    fn resolve_path(&self, path: &str) -> Option<u64> {
        if path == "/tmp" || path == "/Temp" {
            return Some(TMP_INODE);
        }
        let relative = path
            .strip_prefix("/tmp/")
            .or_else(|| path.strip_prefix("/Temp/"))?;
        let mut parent = TMP_INODE;
        for component in relative.split('/') {
            if component.is_empty() {
                return None;
            }
            parent = self.child(parent, component.as_bytes())?.inode;
        }
        Some(parent)
    }

    fn find_inode(&self, inode: u64) -> Option<&WritableNode> {
        self.nodes.iter().find(|node| node.inode == inode)
    }

    fn find_inode_mut(&mut self, inode: u64) -> Option<&mut WritableNode> {
        self.nodes.iter_mut().find(|node| node.inode == inode)
    }

    fn create(&mut self, path: &str, file_type: FileType) -> Result<u64, FsError> {
        let (parent_path, name) = split_parent_name(path)?;
        let parent = self.resolve_path(parent_path).ok_or(FsError::NotFound)?;
        if parent != TMP_INODE
            && self
                .find_inode(parent)
                .is_none_or(|node| node.file_type != FileType::Directory)
        {
            return Err(FsError::NotDirectory);
        }
        if self.child(parent, name).is_some() {
            return Err(FsError::AlreadyExists);
        }
        if self.nodes.len() == MAX_WRITABLE_NODES {
            return Err(FsError::NoSpace);
        }
        let inode = self.next_inode;
        self.next_inode = self.next_inode.checked_add(1).ok_or(FsError::NoSpace)?;
        self.nodes
            .push(WritableNode::new(inode, parent, name, file_type));
        Ok(inode)
    }

    fn write(&mut self, inode: u64, offset: usize, source: &[u8]) -> Result<usize, FsError> {
        let node = self.find_inode_mut(inode).ok_or(FsError::NotFound)?;
        if node.file_type != FileType::File {
            return Err(FsError::IsDirectory);
        }
        let end = offset
            .checked_add(source.len())
            .ok_or(FsError::FileTooLarge)?;
        if end > node.data.len() {
            return Err(FsError::FileTooLarge);
        }
        node.data[offset..end].copy_from_slice(source);
        node.len = node.len.max(end);
        Ok(source.len())
    }

    fn remove(&mut self, path: &str, expected: FileType) -> Result<u64, FsError> {
        let inode = self.resolve_path(path).ok_or(FsError::NotFound)?;
        if inode == TMP_INODE {
            return Err(FsError::ReadOnly);
        }
        let index = self
            .nodes
            .iter()
            .position(|node| node.inode == inode)
            .ok_or(FsError::NotFound)?;
        if self.nodes[index].file_type != expected {
            return if expected == FileType::File {
                Err(FsError::IsDirectory)
            } else {
                Err(FsError::NotDirectory)
            };
        }
        if expected == FileType::Directory && self.nodes.iter().any(|node| node.parent == inode) {
            return Err(FsError::DirectoryNotEmpty);
        }
        Ok(self.nodes.remove(index).inode)
    }

    fn rename(&mut self, old_path: &str, new_path: &str) -> Result<u64, FsError> {
        let inode = self.resolve_path(old_path).ok_or(FsError::NotFound)?;
        if inode == TMP_INODE {
            return Err(FsError::ReadOnly);
        }
        let (new_parent_path, new_name) = split_parent_name(new_path)?;
        let new_parent = self
            .resolve_path(new_parent_path)
            .ok_or(FsError::NotFound)?;
        if new_parent != TMP_INODE
            && self
                .find_inode(new_parent)
                .is_none_or(|node| node.file_type != FileType::Directory)
        {
            return Err(FsError::NotDirectory);
        }
        if self.child(new_parent, new_name).is_some() {
            return Err(FsError::AlreadyExists);
        }
        if self.is_descendant(new_parent, inode) {
            return Err(FsError::InvalidPath);
        }
        let node = self
            .nodes
            .iter_mut()
            .find(|node| node.inode == inode)
            .ok_or(FsError::NotFound)?;
        node.parent = new_parent;
        node.name.fill(0);
        node.name[..new_name.len()].copy_from_slice(new_name);
        node.name_len = new_name.len();
        Ok(node.inode)
    }

    fn is_descendant(&self, mut candidate: u64, ancestor: u64) -> bool {
        while candidate != TMP_INODE {
            if candidate == ancestor {
                return true;
            }
            let Some(node) = self.find_inode(candidate) else {
                return false;
            };
            candidate = node.parent;
        }
        false
    }
}

#[derive(Clone, Copy)]
enum StaticRef {
    BuiltIn(&'static FileEntry),
    Userland(crate::user::images::UserlandImage),
}

impl StaticRef {
    fn inode(self) -> u64 {
        match self {
            Self::BuiltIn(entry) => entry.inode,
            Self::Userland(image) => userland_inode(image.path),
        }
    }

    fn data(self) -> &'static [u8] {
        match self {
            Self::BuiltIn(entry) => entry.data,
            Self::Userland(image) => image.data,
        }
    }
}

fn find_static(path: &str) -> Option<StaticRef> {
    FILES
        .iter()
        .find(|entry| entry.path == path)
        .map(StaticRef::BuiltIn)
        .or_else(|| crate::user::images::find(path).map(StaticRef::Userland))
        .or_else(|| {
            let suffix = path.strip_prefix("/Apps/")?;
            crate::user::images::USERLAND_IMAGES
                .iter()
                .copied()
                .find(|image| image.path.strip_prefix("/bin/") == Some(suffix))
                .map(StaticRef::Userland)
        })
}

fn base_directory(path: &str) -> Option<(u64, &'static str)> {
    let name = path.strip_prefix('/')?;
    if name.contains('/') {
        return None;
    }
    BASE_DIRECTORIES
        .iter()
        .position(|candidate| *candidate == name)
        .map(|index| (BASE_DIRECTORY_INODE + index as u64, BASE_DIRECTORIES[index]))
}

fn static_by_inode(inode: u64) -> Option<StaticRef> {
    FILES
        .iter()
        .find(|entry| entry.inode == inode)
        .map(StaticRef::BuiltIn)
        .or_else(|| {
            crate::user::images::USERLAND_IMAGES
                .iter()
                .copied()
                .find(|image| userland_inode(image.path) == inode)
                .map(StaticRef::Userland)
        })
}

fn userland_inode(path: &str) -> u64 {
    crate::user::images::USERLAND_IMAGES
        .iter()
        .position(|image| image.path == path)
        .map(|index| 1000 + index as u64)
        .unwrap_or(0)
}

fn split_parent_name(path: &str) -> Result<(&str, &[u8]), FsError> {
    if !path.starts_with("/tmp/") && !path.starts_with("/Temp/") {
        return Err(FsError::ReadOnly);
    }
    let split = path.rfind('/').ok_or(FsError::InvalidPath)?;
    let parent = if split == 0 { "/" } else { &path[..split] };
    let name = path[split + 1..].as_bytes();
    if name.is_empty() {
        return Err(FsError::InvalidPath);
    }
    if name.len() > MAX_FILE_NAME_LEN {
        return Err(FsError::NameTooLong);
    }
    Ok((parent, name))
}

#[cfg(test)]
mod tests {
    use super::WritableStore;
    use crate::fs::FsError;

    #[test_case]
    fn writable_store_create_write_rename_and_unlink() {
        let mut store = WritableStore::new();
        let inode = store.create("/tmp/notes", super::FileType::File).unwrap();
        assert_eq!(store.write(inode, 0, b"hello").unwrap(), 5);
        assert_eq!(store.find_inode(inode).unwrap().data[..5], *b"hello");
        assert_eq!(store.rename("/tmp/notes", "/tmp/done").unwrap(), inode);
        assert!(store.resolve_path("/tmp/notes").is_none());
        assert_eq!(store.resolve_path("/tmp/done"), Some(inode));
        assert_eq!(
            store.remove("/tmp/done", super::FileType::File).unwrap(),
            inode
        );
        assert!(store.find_inode(inode).is_none());
    }

    #[test_case]
    fn writable_store_enforces_scope_duplicates_and_size() {
        let mut store = WritableStore::new();
        assert_eq!(
            store.create("/README", super::FileType::File),
            Err(FsError::ReadOnly)
        );
        let inode = store.create("/tmp/a", super::FileType::File).unwrap();
        assert_eq!(
            store.create("/tmp/a", super::FileType::File),
            Err(FsError::AlreadyExists)
        );
        assert_eq!(
            store.write(inode, super::MAX_FILE_SIZE, b"x"),
            Err(FsError::FileTooLarge)
        );
    }

    #[test_case]
    fn writable_store_supports_nested_directories() {
        let mut store = WritableStore::new();
        let docs = store
            .create("/tmp/docs", super::FileType::Directory)
            .unwrap();
        let note = store
            .create("/tmp/docs/note", super::FileType::File)
            .unwrap();
        assert_eq!(store.resolve_path("/tmp/docs"), Some(docs));
        assert_eq!(store.resolve_path("/tmp/docs/note"), Some(note));
        assert_eq!(
            store.remove("/tmp/docs", super::FileType::Directory),
            Err(FsError::DirectoryNotEmpty)
        );
        store
            .remove("/tmp/docs/note", super::FileType::File)
            .unwrap();
        store
            .remove("/tmp/docs", super::FileType::Directory)
            .unwrap();
    }
}
