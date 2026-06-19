use crate::storage::ramdisk::RamDisk;

const MAX_NORMALIZED_PATH_LEN: usize = 128;

static README: &[u8] =
    b"Vantara OS RAM filesystem\n\nThis is a read-only development filesystem.\n";
static VERSION: &[u8] = env!("CARGO_PKG_VERSION").as_bytes();
static MOTD: &[u8] =
    b"Welcome to Vantara OS.\nTry: help, uptime, mem, tasks, pci, ls, cat, stat.\n";
static RAMDISK_BYTES: &[u8] = b"vantara-ramdisk";

static RAMDISK: RamDisk = RamDisk::new(RAMDISK_BYTES);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    pub path: &'static str,
    pub file_type: FileType,
    pub size: usize,
    pub readonly: bool,
    pub inode: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizedPath {
    bytes: [u8; MAX_NORMALIZED_PATH_LEN],
    len: usize,
}

impl NormalizedPath {
    pub const fn root() -> Self {
        let mut bytes = [0; MAX_NORMALIZED_PATH_LEN];
        bytes[0] = b'/';
        Self { bytes, len: 1 }
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

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

pub fn init() {
    crate::serial_println!(
        "[FS] Mounted read-only RAM filesystem: files={} ramdisk_bytes={} blocks={}",
        FILES.len() + crate::user::images::USERLAND_IMAGES.len(),
        RAMDISK.len(),
        crate::storage::block::BlockDevice::block_count(&RAMDISK)
    );
}

pub fn list(path: &str) -> Result<&'static [&'static str], FsError> {
    list_from("/", path)
}

pub fn list_from(cwd: &str, path: &str) -> Result<&'static [&'static str], FsError> {
    let normalized = normalize_path_from(cwd, path)?;
    match normalized.as_str() {
        "/" => {
            static NAMES: &[&str] = &["README", "VERSION", "MOTD", "bin"];
            Ok(NAMES)
        }
        "/bin" => Ok(crate::user::images::USERLAND_IMAGE_NAMES),
        _ => Err(FsError::NotFound),
    }
}

pub fn read(path: &str) -> Result<&'static [u8], FsError> {
    read_from("/", path)
}

pub fn read_from(cwd: &str, path: &str) -> Result<&'static [u8], FsError> {
    match find_from(cwd, path)? {
        FileRef::BuiltIn(entry) => Ok(entry.data),
        FileRef::Userland(image) => Ok(image.data),
    }
}

pub fn stat(path: &str) -> Result<FileStat, FsError> {
    stat_from("/", path)
}

pub fn stat_from(cwd: &str, path: &str) -> Result<FileStat, FsError> {
    let normalized = normalize_path_from(cwd, path)?;
    match normalized.as_str() {
        "/" => Ok(FileStat {
            path: "/",
            file_type: FileType::Directory,
            size: 4,
            readonly: true,
            inode: 1,
        }),
        "/bin" => Ok(FileStat {
            path: "/bin",
            file_type: FileType::Directory,
            size: crate::user::images::USERLAND_IMAGE_NAMES.len(),
            readonly: true,
            inode: 2,
        }),
        _ => match find_normalized(normalized.as_str())? {
            FileRef::BuiltIn(entry) => Ok(FileStat {
                path: entry.path,
                file_type: FileType::File,
                size: entry.data.len(),
                readonly: true,
                inode: entry.inode,
            }),
            FileRef::Userland(image) => Ok(FileStat {
                path: image.path,
                file_type: FileType::File,
                size: image.data.len(),
                readonly: true,
                inode: userland_inode(image.path),
            }),
        },
    }
}

fn find_from(cwd: &str, path: &str) -> Result<FileRef, FsError> {
    let path = normalize_path_from(cwd, path)?;
    find_normalized(path.as_str())
}

fn find_normalized(path: &str) -> Result<FileRef, FsError> {
    if let Some(entry) = FILES.iter().find(|entry| entry.path == path) {
        return Ok(FileRef::BuiltIn(entry));
    }

    if let Some(image) = crate::user::images::find(path) {
        return Ok(FileRef::Userland(image));
    }

    Err(FsError::NotFound)
}

pub fn normalize_path(path: &str) -> Result<NormalizedPath, FsError> {
    normalize_path_from("/", path)
}

pub fn normalize_path_from(cwd: &str, path: &str) -> Result<NormalizedPath, FsError> {
    let mut normalized = NormalizedPath {
        bytes: [0; MAX_NORMALIZED_PATH_LEN],
        len: 0,
    };
    push_root(&mut normalized)?;

    if !path.starts_with('/') {
        push_path_components(&mut normalized, cwd)?;
    }
    push_path_components(&mut normalized, path)?;

    Ok(normalized)
}

fn push_path_components(out: &mut NormalizedPath, path: &str) -> Result<(), FsError> {
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => pop_component(out),
            name => push_component(out, name.as_bytes())?,
        }
    }

    Ok(())
}

fn push_root(out: &mut NormalizedPath) -> Result<(), FsError> {
    out.bytes[0] = b'/';
    out.len = 1;
    Ok(())
}

fn push_component(out: &mut NormalizedPath, component: &[u8]) -> Result<(), FsError> {
    if component.is_empty() {
        return Ok(());
    }
    if component.iter().any(|byte| *byte == 0) {
        return Err(FsError::InvalidPath);
    }

    let needs_slash = out.len > 1;
    let needed = out
        .len
        .checked_add(needs_slash as usize)
        .and_then(|len| len.checked_add(component.len()))
        .ok_or(FsError::PathTooLong)?;
    if needed > out.bytes.len() {
        return Err(FsError::PathTooLong);
    }

    if needs_slash {
        out.bytes[out.len] = b'/';
        out.len += 1;
    }
    out.bytes[out.len..out.len + component.len()].copy_from_slice(component);
    out.len += component.len();
    Ok(())
}

fn pop_component(out: &mut NormalizedPath) {
    if out.len <= 1 {
        return;
    }

    while out.len > 1 && out.bytes[out.len - 1] != b'/' {
        out.len -= 1;
    }
    if out.len > 1 {
        out.len -= 1;
    }
}

fn userland_inode(path: &str) -> u64 {
    crate::user::images::USERLAND_IMAGES
        .iter()
        .position(|image| image.path == path)
        .map(|index| 1000 + index as u64)
        .unwrap_or(0)
}

enum FileRef {
    BuiltIn(&'static FileEntry),
    Userland(crate::user::images::UserlandImage),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    NotDirectory,
    InvalidPath,
    PathTooLong,
}

#[cfg(test)]
mod tests {
    use super::{FileType, FsError, list, normalize_path, normalize_path_from, read, stat};

    #[test_case]
    fn root_filesystem_contract_is_stable() {
        assert_eq!(list("/").unwrap(), &["README", "VERSION", "MOTD", "bin"]);
        assert!(
            read("/README")
                .unwrap()
                .starts_with(b"Vantara OS RAM filesystem")
        );

        let bin = stat("/bin").unwrap();
        assert_eq!(bin.file_type, FileType::Directory);
        assert!(bin.readonly);
    }

    #[test_case]
    fn normalizes_dot_dot_and_repeated_slashes() {
        assert_eq!(
            normalize_path("////bin//../README").unwrap().as_str(),
            "/README"
        );
        assert_eq!(normalize_path("./bin/./ls").unwrap().as_str(), "/bin/ls");
        assert_eq!(normalize_path("/../../").unwrap().as_str(), "/");
    }

    #[test_case]
    fn normalizes_relative_paths_from_cwd() {
        assert_eq!(
            normalize_path_from("/bin", "../README").unwrap().as_str(),
            "/README"
        );
        assert_eq!(normalize_path_from("/", "MOTD").unwrap().as_str(), "/MOTD");
    }

    #[test_case]
    fn rejects_overlong_paths() {
        let long_path = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert_eq!(normalize_path(long_path), Err(FsError::PathTooLong));
    }
}
