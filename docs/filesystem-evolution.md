# Vantara Filesystem Evolution

This document records the next filesystem shape after the current read-only RAM
filesystem.

## Current Model

- Root directory: `/`
- Built-in read-only files: `/README`, `/VERSION`, `/MOTD`
- Built-in read-only executable directory: `/bin`
- User programs are exposed as read-only files under `/bin`
- Paths are normalized before lookup:
  - `.`
  - `..`
  - repeated `/`
  - relative paths against a process current working directory

## Writable RAM Filesystem

The first writable slice is implemented. The kernel now exposes a real mutable
node store beneath `/tmp`, while `/`, `/bin`, and built-in files remain
read-only.

The next filesystem layer should add mutable RAM-backed files without weakening
the read-only built-ins.

Node model:

```text
FsNode {
    inode: u64
    name: fixed bytes
    parent: inode
    file_type: file | directory
    readonly: bool
    data: fixed-capacity byte buffer for regular files
}
```

Current limits:

- Maximum writable files: 32
- Maximum file name length: 32 bytes
- Maximum file size: 1024 bytes
- Nested writable directories are supported
- Writable RAM nodes live under `/tmp`

Each open regular-file descriptor stores a VFS node ID and offset. Reads and
writes therefore access live filesystem data instead of a snapshot captured at
open time. Directory reads build a bounded listing from the selected backend.

## VFS Layer

Filesystem consumers now call a virtual filesystem layer instead of the RAM
backend directly. The VFS owns:

- normalized absolute-path dispatch;
- a mount table with longest-prefix matching;
- translation from global paths to backend-relative paths;
- globally unique node identity as `(mount_id, inode)`;
- cross-backend operation checks.

The root RAM filesystem is mounted at `/` as mount ID 1. Open descriptors retain
the mount ID together with the backend inode, so future filesystems may reuse
their own inode numbering safely. Rename is currently allowed only within one
mount; a cross-mount rename returns `CrossDevice`.

Backend contract:

```text
FilesystemBackend {
    list(path, out)
    stat(path)
    read(inode, offset, out)
    write(inode, offset, data)
    create(path)
    mkdir(path)
    unlink(path)
    rmdir(path)
    rename(old_path, new_path)
}
```

This boundary is ready for `/dev`, initrd, and a future block-backed filesystem
without changing syscall or process code.

## Persistent VANTFS Volume

Vantara now mounts a dedicated block-backed filesystem at `/persist` when a
QEMU IDE primary-slave disk is present. The initial driver uses ATA PIO in
polling mode with device interrupts disabled.

The volume format is deliberately small and versioned:

- magic: `VANTFS01`
- format version: 1
- 512-byte sectors
- 32 fixed directory entries
- 32-byte maximum file name
- 1024-byte maximum file size
- two data sectors reserved per file
- inode-linked directory tree rooted at `/persist`

The dedicated persistence disk is automatically formatted only when its first
sector does not contain the VANTFS magic. Existing VANTFS data is mounted
without reformatting. A missing second disk leaves the backend unavailable and
does not expose `/persist`.

Example QEMU attachment:

```text
-drive format=raw,file=bootimage-kernel.bin,index=0,media=disk
-drive format=raw,file=vantara-persist.img,index=1,media=disk
```

The regression test boots twice with the same disk image:

```text
boot 1: mkdir /persist/docs
        write /persist/docs/hello forever
boot 2: cat /persist/docs/hello
```

The second boot must report that it mounted an existing volume and print
`forever`.

## Create File

Implemented syscall:

```text
SYS_CREATE path_ptr, path_len -> fd or error
```

Rules:

- Normalize `path` against the caller cwd.
- Select the writable backend through VFS (`/tmp` or `/persist`).
- Reject paths that already exist.
- Allocate a new inode and empty mutable file.
- Return an open fd positioned at offset 0.

## Directories

Implemented syscalls:

```text
SYS_MKDIR path_ptr, path_len -> 0 or error
SYS_RMDIR path_ptr, path_len -> 0 or error
```

Rules:

- Parent directories must already exist.
- File and directory names share one namespace under each parent.
- `rmdir` rejects non-empty directories.
- Filesystem roots such as `/tmp` and `/persist` cannot be removed.
- `rename` may move files or directories within the same mounted backend.
- Moving a directory beneath its own descendant is rejected.

## Write File

Implemented through the existing syscall:

```text
SYS_WRITE fd, ptr, len -> bytes written or error
```

Rules:

- Only file descriptors opened for mutable files are writable.
- Writes append or replace from the descriptor offset.
- Reject writes past maximum file size.
- Keep `SYS_WRITE` to stdout/stderr behavior unchanged.

## Delete File

Implemented syscall:

```text
SYS_UNLINK path_ptr, path_len -> 0 or error
```

Rules:

- Normalize path against cwd.
- Reject read-only built-ins.
- Reject directories.
- Reject open-file deletion at first.

## Rename File

Implemented syscall:

```text
SYS_RENAME old_ptr, old_len, new_ptr, new_len -> 0 or error
```

Rules:

- Normalize both paths.
- Reject read-only built-ins.
- Keep rename within `/tmp` at first.
- Reject target if it already exists.

## Userland Commands

Implemented commands:

- `/bin/touch`
- `/bin/write`
- `/bin/rm`
- `/bin/mv`

The existing `/bin/ls`, `/bin/cat`, and `/bin/stat` should work across both
read-only built-ins and writable RAM files.

## Next Evolution

- Extend `devfs` beyond `null`, `zero`, and live registry views as character
  and block driver interfaces mature.
- Add timestamps, link count, ownership, and permission metadata.
- Add a block cache, allocation bitmap, and crash-consistent metadata updates.
