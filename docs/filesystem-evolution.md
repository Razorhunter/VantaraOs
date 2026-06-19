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

## Writable RAM Filesystem Target

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

Initial limits:

- Maximum writable files: 32
- Maximum file name length: 32 bytes
- Maximum file size: 1024 bytes
- No nested writable directories at first
- Writable files live under `/tmp`

## Create File

Planned syscall:

```text
SYS_CREATE path_ptr, path_len -> fd or error
```

Rules:

- Normalize `path` against the caller cwd.
- Reject paths outside `/tmp`.
- Reject paths that already exist.
- Allocate a new inode and empty mutable file.
- Return an open fd positioned at offset 0.

## Write File

Planned syscall:

```text
SYS_WRITE_FD fd, ptr, len -> bytes written or error
```

Rules:

- Only file descriptors opened for mutable files are writable.
- Writes append or replace from the descriptor offset.
- Reject writes past maximum file size.
- Keep `SYS_WRITE` to stdout/stderr behavior unchanged.

## Delete File

Planned syscall:

```text
SYS_UNLINK path_ptr, path_len -> 0 or error
```

Rules:

- Normalize path against cwd.
- Reject read-only built-ins.
- Reject directories.
- Reject open-file deletion at first.

## Rename File

Planned syscall:

```text
SYS_RENAME old_ptr, old_len, new_ptr, new_len -> 0 or error
```

Rules:

- Normalize both paths.
- Reject read-only built-ins.
- Keep rename within `/tmp` at first.
- Reject target if it already exists.

## Userland Commands

First commands to port once syscalls exist:

- `/bin/touch`
- `/bin/write`
- `/bin/rm`
- `/bin/mv`

The existing `/bin/ls`, `/bin/cat`, and `/bin/stat` should work across both
read-only built-ins and writable RAM files.
