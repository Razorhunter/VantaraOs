mod devfs;
mod persistent;
mod ramfs;

pub const MAX_NORMALIZED_PATH_LEN: usize = 128;

const ROOT_MOUNT_ID: MountId = MountId(1);
const PERSISTENT_MOUNT_ID: MountId = MountId(2);
const DEVICE_MOUNT_ID: MountId = MountId(3);
const DATA_MOUNT_ID: MountId = MountId(4);
const DEVICES_MOUNT_ID: MountId = MountId(5);
static RAM_FILESYSTEM: ramfs::RamFilesystem = ramfs::RamFilesystem::new();
static PERSISTENT_FILESYSTEM: persistent::PersistentFilesystem =
    persistent::PersistentFilesystem::new();
static DEVICE_FILESYSTEM: devfs::DeviceFilesystem = devfs::DeviceFilesystem::new();
static ROOT_MOUNTS: &[Mount] = &[
    Mount {
        id: ROOT_MOUNT_ID,
        path: "/",
        backend: &RAM_FILESYSTEM,
    },
    Mount {
        id: PERSISTENT_MOUNT_ID,
        path: "/persist",
        backend: &PERSISTENT_FILESYSTEM,
    },
    Mount {
        id: DEVICE_MOUNT_ID,
        path: "/dev",
        backend: &DEVICE_FILESYSTEM,
    },
    Mount {
        id: DATA_MOUNT_ID,
        path: "/Data",
        backend: &PERSISTENT_FILESYSTEM,
    },
    Mount {
        id: DEVICES_MOUNT_ID,
        path: "/Devices",
        backend: &DEVICE_FILESYSTEM,
    },
];
static VFS: VirtualFileSystem = VirtualFileSystem::new(ROOT_MOUNTS);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    pub path: NormalizedPath,
    pub file_type: FileType,
    pub size: usize,
    pub readonly: bool,
    pub node: VfsNodeId,
}

impl FileStat {
    pub const fn inode(self) -> u64 {
        self.node.inode
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenTarget {
    pub path: NormalizedPath,
    pub file_type: FileType,
    pub readonly: bool,
    pub node: VfsNodeId,
}

impl OpenTarget {
    pub const fn inode(self) -> u64 {
        self.node.inode
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VfsNodeId {
    pub mount: MountId,
    pub inode: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MountId(u16);

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

impl core::fmt::Display for NormalizedPath {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BackendStat {
    pub file_type: FileType,
    pub size: usize,
    pub readonly: bool,
    pub inode: u64,
}

pub(crate) trait FilesystemBackend: Sync {
    fn name(&self) -> &'static str;
    fn available(&self) -> bool {
        true
    }
    fn list(&self, path: &str, out: &mut [u8]) -> Result<usize, FsError>;
    fn stat(&self, path: &str) -> Result<BackendStat, FsError>;
    fn read(&self, inode: u64, offset: usize, out: &mut [u8]) -> Result<usize, FsError>;
    fn write(&self, inode: u64, offset: usize, source: &[u8]) -> Result<usize, FsError>;
    fn create(&self, path: &str) -> Result<BackendStat, FsError>;
    fn mkdir(&self, path: &str) -> Result<BackendStat, FsError>;
    fn unlink(&self, path: &str) -> Result<u64, FsError>;
    fn rmdir(&self, path: &str) -> Result<u64, FsError>;
    fn rename(&self, old_path: &str, new_path: &str) -> Result<u64, FsError>;
}

struct Mount {
    id: MountId,
    path: &'static str,
    backend: &'static dyn FilesystemBackend,
}

struct ResolvedMount<'a> {
    mount: &'a Mount,
    backend_path: &'a str,
}

struct VirtualFileSystem {
    mounts: &'static [Mount],
}

impl VirtualFileSystem {
    const fn new(mounts: &'static [Mount]) -> Self {
        Self { mounts }
    }

    fn resolve<'a>(&'a self, path: &'a str) -> Result<ResolvedMount<'a>, FsError> {
        let mount = self
            .mounts
            .iter()
            .filter(|mount| mount.backend.available() && path_matches_mount(path, mount.path))
            .max_by_key(|mount| mount.path.len())
            .ok_or(FsError::NotFound)?;
        Ok(ResolvedMount {
            mount,
            backend_path: strip_mount_path(path, mount.path),
        })
    }

    fn mount(&self, id: MountId) -> Result<&Mount, FsError> {
        self.mounts
            .iter()
            .find(|mount| mount.id == id)
            .ok_or(FsError::StaleNode)
    }

    fn stat(&self, path: NormalizedPath) -> Result<FileStat, FsError> {
        let resolved = self.resolve(path.as_str())?;
        let stat = resolved.mount.backend.stat(resolved.backend_path)?;
        Ok(FileStat {
            path,
            file_type: stat.file_type,
            size: stat.size,
            readonly: stat.readonly,
            node: VfsNodeId {
                mount: resolved.mount.id,
                inode: stat.inode,
            },
        })
    }
}

pub fn init() {
    RAM_FILESYSTEM.init();
    PERSISTENT_FILESYSTEM.init();
    DEVICE_FILESYSTEM.init();
    crate::serial_println!(
        "[VFS] mounted backend={} path=/ mount_id={}",
        RAM_FILESYSTEM.name(),
        ROOT_MOUNT_ID.0
    );
    if PERSISTENT_FILESYSTEM.available() {
        crate::serial_println!(
            "[VFS] mounted backend={} path=/Data mount_id={}",
            PERSISTENT_FILESYSTEM.name(),
            DATA_MOUNT_ID.0
        );
        crate::serial_println!(
            "[VFS] mounted backend={} path=/persist mount_id={}",
            PERSISTENT_FILESYSTEM.name(),
            PERSISTENT_MOUNT_ID.0
        );
    }
    crate::serial_println!(
        "[VFS] mounted backend={} path=/Devices mount_id={}",
        DEVICE_FILESYSTEM.name(),
        DEVICES_MOUNT_ID.0
    );
    crate::serial_println!(
        "[VFS] mounted backend={} path=/dev mount_id={}",
        DEVICE_FILESYSTEM.name(),
        DEVICE_MOUNT_ID.0
    );
}

#[cfg(feature = "ahci-vantfs-test")]
pub fn run_ahci_vantfs_test() {
    persistent::run_ahci_backend_test();
}

pub fn list_to_buffer(cwd: &str, path: &str, out: &mut [u8]) -> Result<usize, FsError> {
    let normalized = normalize_path_from(cwd, path)?;
    let resolved = VFS.resolve(normalized.as_str())?;
    let mut written = resolved.mount.backend.list(resolved.backend_path, out)?;
    for mount in VFS.mounts {
        if !mount.backend.available() {
            continue;
        }
        if let Some(name) = direct_mount_child(normalized.as_str(), mount.path) {
            if !contains_line(&out[..written], name.as_bytes()) {
                written = append_line(out, written, name.as_bytes())?;
            }
        }
    }
    Ok(written)
}

fn contains_line(listing: &[u8], candidate: &[u8]) -> bool {
    listing
        .split(|byte| *byte == b'\n')
        .any(|line| line == candidate)
}

pub fn read_to_buffer(cwd: &str, path: &str, out: &mut [u8]) -> Result<usize, FsError> {
    let target = open_from(cwd, path)?;
    if target.file_type != FileType::File {
        return Err(FsError::IsDirectory);
    }
    read_node(target.node, 0, out)
}

pub fn read_node(node: VfsNodeId, offset: usize, out: &mut [u8]) -> Result<usize, FsError> {
    VFS.mount(node.mount)?.backend.read(node.inode, offset, out)
}

pub fn write_node(node: VfsNodeId, offset: usize, source: &[u8]) -> Result<usize, FsError> {
    VFS.mount(node.mount)?
        .backend
        .write(node.inode, offset, source)
}

pub fn stat(path: &str) -> Result<FileStat, FsError> {
    stat_from("/", path)
}

pub fn stat_from(cwd: &str, path: &str) -> Result<FileStat, FsError> {
    VFS.stat(normalize_path_from(cwd, path)?)
}

pub fn open_from(cwd: &str, path: &str) -> Result<OpenTarget, FsError> {
    let stat = stat_from(cwd, path)?;
    Ok(OpenTarget {
        path: stat.path,
        file_type: stat.file_type,
        readonly: stat.readonly,
        node: stat.node,
    })
}

pub fn create_from(cwd: &str, path: &str) -> Result<OpenTarget, FsError> {
    let normalized = normalize_path_from(cwd, path)?;
    let resolved = VFS.resolve(normalized.as_str())?;
    let stat = resolved.mount.backend.create(resolved.backend_path)?;
    Ok(OpenTarget {
        path: normalized,
        file_type: stat.file_type,
        readonly: stat.readonly,
        node: VfsNodeId {
            mount: resolved.mount.id,
            inode: stat.inode,
        },
    })
}

pub fn mkdir_from(cwd: &str, path: &str) -> Result<OpenTarget, FsError> {
    let normalized = normalize_path_from(cwd, path)?;
    let resolved = VFS.resolve(normalized.as_str())?;
    let stat = resolved.mount.backend.mkdir(resolved.backend_path)?;
    Ok(OpenTarget {
        path: normalized,
        file_type: stat.file_type,
        readonly: stat.readonly,
        node: VfsNodeId {
            mount: resolved.mount.id,
            inode: stat.inode,
        },
    })
}

pub fn unlink_from(cwd: &str, path: &str) -> Result<u64, FsError> {
    let normalized = normalize_path_from(cwd, path)?;
    let resolved = VFS.resolve(normalized.as_str())?;
    resolved.mount.backend.unlink(resolved.backend_path)
}

pub fn rmdir_from(cwd: &str, path: &str) -> Result<u64, FsError> {
    let normalized = normalize_path_from(cwd, path)?;
    let resolved = VFS.resolve(normalized.as_str())?;
    resolved.mount.backend.rmdir(resolved.backend_path)
}

pub fn rename_from(cwd: &str, old_path: &str, new_path: &str) -> Result<u64, FsError> {
    let old_path = normalize_path_from(cwd, old_path)?;
    let new_path = normalize_path_from(cwd, new_path)?;
    let old_mount = VFS.resolve(old_path.as_str())?;
    let new_mount = VFS.resolve(new_path.as_str())?;
    if old_mount.mount.id != new_mount.mount.id {
        return Err(FsError::CrossDevice);
    }
    old_mount
        .mount
        .backend
        .rename(old_mount.backend_path, new_mount.backend_path)
}

pub fn normalize_path(path: &str) -> Result<NormalizedPath, FsError> {
    normalize_path_from("/", path)
}

pub fn normalize_path_from(cwd: &str, path: &str) -> Result<NormalizedPath, FsError> {
    let mut normalized = NormalizedPath {
        bytes: [0; MAX_NORMALIZED_PATH_LEN],
        len: 0,
    };
    push_root(&mut normalized);
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

fn push_root(out: &mut NormalizedPath) {
    out.bytes[0] = b'/';
    out.len = 1;
}

fn push_component(out: &mut NormalizedPath, component: &[u8]) -> Result<(), FsError> {
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

fn path_matches_mount(path: &str, mount_path: &str) -> bool {
    mount_path == "/"
        || path == mount_path
        || path
            .strip_prefix(mount_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn strip_mount_path<'a>(path: &'a str, mount_path: &str) -> &'a str {
    if mount_path == "/" {
        return path;
    }
    match path.strip_prefix(mount_path).unwrap_or(path) {
        "" => "/",
        suffix => suffix,
    }
}

fn direct_mount_child<'a>(directory: &str, mount_path: &'a str) -> Option<&'a str> {
    if mount_path == "/" {
        return None;
    }
    let suffix = if directory == "/" {
        mount_path.strip_prefix('/')?
    } else {
        mount_path.strip_prefix(directory)?.strip_prefix('/')?
    };
    (!suffix.is_empty() && !suffix.contains('/')).then_some(suffix)
}

pub(crate) fn append_line(out: &mut [u8], offset: usize, value: &[u8]) -> Result<usize, FsError> {
    let end = offset
        .checked_add(value.len() + 1)
        .ok_or(FsError::BufferTooSmall)?;
    if end > out.len() {
        return Err(FsError::BufferTooSmall);
    }
    out[offset..offset + value.len()].copy_from_slice(value);
    out[end - 1] = b'\n';
    Ok(end)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    NotDirectory,
    IsDirectory,
    InvalidPath,
    PathTooLong,
    NameTooLong,
    AlreadyExists,
    ReadOnly,
    NoSpace,
    FileTooLarge,
    BufferTooSmall,
    CrossDevice,
    StaleNode,
    Unavailable,
    Corrupt,
    Io,
    DirectoryNotEmpty,
}

#[cfg(test)]
mod tests {
    use super::{
        BackendStat, FileType, FilesystemBackend, FsError, Mount, MountId, NormalizedPath,
        VirtualFileSystem, normalize_path, normalize_path_from, path_matches_mount,
        strip_mount_path,
    };

    struct TestBackend {
        inode: u64,
    }

    impl FilesystemBackend for TestBackend {
        fn name(&self) -> &'static str {
            "testfs"
        }

        fn list(&self, _path: &str, _out: &mut [u8]) -> Result<usize, FsError> {
            Ok(0)
        }

        fn stat(&self, _path: &str) -> Result<BackendStat, FsError> {
            Ok(BackendStat {
                file_type: FileType::Directory,
                size: 0,
                readonly: true,
                inode: self.inode,
            })
        }

        fn read(&self, _inode: u64, _offset: usize, _out: &mut [u8]) -> Result<usize, FsError> {
            Ok(0)
        }

        fn write(&self, _inode: u64, _offset: usize, _source: &[u8]) -> Result<usize, FsError> {
            Err(FsError::ReadOnly)
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

    static ROOT: TestBackend = TestBackend { inode: 1 };
    static DEV: TestBackend = TestBackend { inode: 2 };
    static TEST_MOUNTS: &[Mount] = &[
        Mount {
            id: MountId(1),
            path: "/",
            backend: &ROOT,
        },
        Mount {
            id: MountId(2),
            path: "/dev",
            backend: &DEV,
        },
    ];

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

    #[test_case]
    fn mount_matching_respects_component_boundaries() {
        assert!(path_matches_mount("/dev", "/dev"));
        assert!(path_matches_mount("/dev/input", "/dev"));
        assert!(!path_matches_mount("/device", "/dev"));
        assert_eq!(strip_mount_path("/dev/input", "/dev"), "/input");
    }

    #[test_case]
    fn direct_mount_children_are_exposed_by_parent_directory() {
        assert_eq!(super::direct_mount_child("/", "/persist"), Some("persist"));
        assert_eq!(
            super::direct_mount_child("/dev", "/dev/input"),
            Some("input")
        );
        assert_eq!(super::direct_mount_child("/", "/dev/input"), None);
    }

    #[test_case]
    fn resolver_prefers_longest_matching_mount() {
        let vfs = VirtualFileSystem::new(TEST_MOUNTS);
        let root = vfs.resolve("/README").unwrap();
        let dev = vfs.resolve("/dev/keyboard").unwrap();
        assert_eq!(root.mount.id, MountId(1));
        assert_eq!(root.backend_path, "/README");
        assert_eq!(dev.mount.id, MountId(2));
        assert_eq!(dev.backend_path, "/keyboard");
    }

    #[test_case]
    fn vfs_node_identity_includes_mount() {
        let vfs = VirtualFileSystem::new(TEST_MOUNTS);
        let root = vfs.stat(NormalizedPath::root()).unwrap();
        let dev_path = normalize_path("/dev").unwrap();
        let dev = vfs.stat(dev_path).unwrap();
        assert_eq!(root.node.inode, 1);
        assert_eq!(dev.node.inode, 2);
        assert_ne!(root.node.mount, dev.node.mount);
    }
}
