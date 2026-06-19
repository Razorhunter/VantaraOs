# Vantara Syscall ABI

This document defines **Vantara Native ABI v1.1** for Ring-3 programs. ABI v1 is
the compatibility contract between the kernel and native Vantara userland.
Userland commands should use the shared wrapper in
`userland/native/src/abi.rs` instead of open-coding `int 0x80`.

## Version And Compatibility Policy

- ABI version encoding is `(major << 32) | minor`.
- `SYS_ABI_INFO` returns the kernel-supported ABI version.
- A major-version change may break existing binaries and requires an explicit
  migration plan.
- A minor-version change must be backward compatible with all earlier minors
  of the same major.
- Published syscall numbers and negative error values are never reassigned.
- New syscalls use new numbers appended after the current highest number.
- Existing structures may only grow through a new syscall, an explicit size
  argument, or reserved fields; existing field offsets do not change.
- Unknown syscall numbers return `ERR_UNKNOWN_SYSCALL`.
- Kernel changes must pass `make abi-check` and relevant syscall tests.

## Trap Interface

- Trap instruction: `int 0x80`
- Syscall number: `rax`
- Arguments:
  - `arg0`: `rdi`
  - `arg1`: `rsi`
  - `arg2`: `rdx`
  - `arg3`: `r10`
  - `arg4`: `r8`
  - `arg5`: `r9`
- Return value: `rax`
- Error convention: negative `i64` values returned in `rax`

The kernel preserves and can resume full user register context across blocking
or yielding syscalls. Userland should still treat `rax` as the return register
and should not rely on scratch-register contents after a syscall unless the
shared wrapper documents that behavior.

Every pointer/length pair is validated against the current process memory
layout and then checked page-by-page in the active process page table. Each page
must be present and user-accessible; kernel output buffers must additionally be
writable. Validation rejects null, empty, overflowing, unmapped, cross-boundary,
kernel-space, and ownerless ranges before dereferencing them.

## Syscall Numbers

| Number | Name | Arguments | Return |
| --- | --- | --- | --- |
| 1 | `SYS_EXIT` | `status` | Does not return |
| 2 | `SYS_WRITE` | `fd`, `ptr`, `len` | bytes written or error |
| 3 | `SYS_UPTIME` | `out_ptr`, `out_len` | uptime ms or bytes copied |
| 4 | `SYS_GETUID` | none | uid |
| 5 | `SYS_GETGID` | none | gid |
| 6 | `SYS_WHOAMI` | `out_ptr`, `out_len` | bytes copied |
| 7 | `SYS_LISTDIR` | `path_ptr`, `path_len`, `out_ptr`, `out_len` | bytes copied |
| 8 | `SYS_READ_FILE` | `path_ptr`, `path_len`, `out_ptr`, `out_len` | bytes copied |
| 9 | `SYS_STAT` | `path_ptr`, `path_len`, `out_ptr`, `out_len` | bytes copied or error |
| 10 | `SYS_OPEN` | `path_ptr`, `path_len` | fd or error |
| 11 | `SYS_READ` | `fd`, `out_ptr`, `out_len` | bytes read or error |
| 12 | `SYS_CLOSE` | `fd` | 0 or error |
| 13 | `SYS_EXEC` | `path_ptr`, `path_len`, `arg_ptr`, `arg_len` | child pid or error |
| 14 | `SYS_WAITPID` | `pid` | child exit status or error |
| 15 | `SYS_PROCS` | `out_ptr`, `out_len` | bytes copied |
| 16 | `SYS_KILL` | `pid`, `signal` | 0 or error |
| 17 | `SYS_YIELD` | ignored | 0 or scheduler handoff |
| 18 | `SYS_EXEC_BG` | `path_ptr`, `path_len`, `arg_ptr`, `arg_len` | child pid or error |
| 19 | `SYS_SLEEP_MS` | `duration_ms` | 0 after wake or error |
| 20 | `SYS_GETCWD` | `out_ptr`, `out_len` | bytes copied |
| 21 | `SYS_CHDIR` | `path_ptr`, `path_len` | 0 or error |
| 22 | `SYS_SETUSER` | `name_ptr`, `name_len` | 0 or error |
| 23 | `SYS_REBOOT` | none | Does not return |
| 24 | `SYS_PCI_LIST` | `out_ptr`, `out_len` | bytes copied |
| 25 | `SYS_NETDEV_LIST` | `out_ptr`, `out_len` | bytes copied |
| 26 | `SYS_KLOG_READ` | `out_ptr`, `out_len` | bytes copied |
| 27 | `SYS_DRIVER_STATUS` | `out_ptr`, `out_len` | bytes copied |
| 28 | `SYS_ABI_INFO` | none | ABI version `(major << 32) | minor` |
| 29 | `SYS_THREAD_CREATE` | entry, stack pointer, stack length, argument | new TID or negative error |
| 30 | `SYS_THREAD_EXIT` | none | does not return |

`SYS_THREAD_CREATE` starts a thread in the caller's process and address space.
The caller supplies a writable, non-overlapping stack buffer of at least 1024
bytes. The new thread begins at `entry` with `argument` in `RDI`. Its entry
function must terminate through `SYS_THREAD_EXIT`; returning directly is not
supported yet.

## Stable Error Values

| Value | Name | Meaning |
| --- | --- | --- |
| -1 | `ERR_UNKNOWN_SYSCALL` | Syscall number is not recognized |
| -2 | `ERR_INVALID_ARGUMENT` | Invalid argument, pointer, length, or state |
| -3 | `ERR_NOT_IMPLEMENTED` | Reserved operation is not implemented |
| -4 | `ERR_NO_SUCH_PROCESS` | Process does not exist |
| -5 | `ERR_NOT_CHILD` | Process exists but is not a child of the caller |
| -6 | `ERR_WOULD_BLOCK` | Operation cannot complete without blocking |

## File Metadata

`SYS_STAT` writes this packed user structure when `out_len` is large enough:

```text
u64 size
u64 readonly
u64 file_type   # 1=file, 2=directory
u64 inode       # stable placeholder id for the read-only RAM filesystem
```

## File Descriptors

- `0`: stdin
- `1`: stdout
- `2`: stderr
- `3+`: process-owned descriptors returned by `SYS_OPEN`

`SYS_READ` on stdin may return `-6` (`WouldBlock`) when no input is available.
Userland shells should yield after repeated `WouldBlock` reads.

## Process Arguments

The process argument handoff now has two layers:

Compatibility fixed argument slot:

- The kernel copies the exec argument string into the fixed user argument area.
- The argument bytes live at `0x1000340`.
- The argument length lives at `0x1000378`.
- The current maximum argument byte length is 32 bytes.
- Existing userland code can keep using `program_arg`, `argv`, or `first_arg`
  from the shared wrapper.

Initial user stack:

- The kernel seeds a compact initial stack before the first Ring-3 jump.
- `rsp + 0` contains `argc`.
- `rsp + 8..` contains `argv` pointers.
- `argv[argc]` is null.
- `envp[0]` is null for now.
- The auxiliary vector currently ends immediately with `AT_NULL, 0`.
- `argv[0]` is the program path, and the exec argument string is tokenized on
  spaces/tabs into `argv[1..]`.

## Shared Wrapper

The current libc-lite surface lives in `userland/native/src/abi.rs`:

- `write`, `write_line`, `write_bytes`
- `read`, `open`, `close`
- `exec`, `exec_background`, `waitpid`
- `exit`, `yield_now`
- `sleep_ms`
- `getcwd`, `chdir`
- `uptime_ms`, `whoami`, `listdir`, `read_file`, `stat`, `procs`, `kill`
- `file_stat`
- `program_arg`, `argv`, `first_arg`

New Rust ELF commands should import or share this wrapper instead of defining
their own syscall constants and inline assembly.
