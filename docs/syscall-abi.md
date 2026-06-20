# Vantara Syscall ABI

This document defines **Vantara Native ABI v1.10** for Ring-3 programs. ABI v1 is
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
| 16 | `SYS_KILL` | `pid`, `signal` | 0 delivered, 1 pending, or error |
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
| 31 | `SYS_THREAD_JOIN` | TID | 0 after target exits, or negative error |
| 32 | `SYS_THREAD_SPAWN` | entry, argument | new TID or negative error |
| 33 | `SYS_PIPE` | `out_fds_ptr`, `out_fds_len` | 0 or negative error |
| 34 | `SYS_EVENT_CREATE` | none | event handle or negative error |
| 35 | `SYS_EVENT_WAIT` | event handle | 0 after wake, or negative error |
| 36 | `SYS_EVENT_SIGNAL` | event handle | 1 if a waiter woke, 0 if latched |
| 37 | `SYS_EVENT_CLOSE` | event handle | 0 or negative error |
| 38 | `SYS_MSGQ_CREATE` | none | queue handle or negative error |
| 39 | `SYS_MSGQ_SEND` | queue handle, message pointer, message length | bytes queued or error |
| 40 | `SYS_MSGQ_RECV` | queue handle, output pointer, output length | message bytes or error |
| 41 | `SYS_MSGQ_CLOSE` | queue handle | 0 or negative error |
| 42 | `SYS_SIGPROCMASK` | operation, signal mask | previous mask or negative error |
| 43 | `SYS_SIGPENDING` | none | pending signal mask or negative error |
| 44 | `SYS_SIGACTION` | signal, disposition/handler entry | previous disposition or error |
| 45 | `SYS_SIGRETURN` | none | restores interrupted context; does not return directly |
| 46 | `SYS_GETPGRP` | none | caller process-group ID |
| 47 | `SYS_SETPGID` | pid or 0, process-group ID or 0 | resulting process-group ID |
| 48 | `SYS_GETSID` | pid or 0 | session ID |
| 49 | `SYS_SETSID` | none | new session ID or error |

`SYS_THREAD_CREATE` starts a thread in the caller's process and address space.
The caller supplies a writable, non-overlapping stack buffer of at least 1024
bytes. The new thread begins at `entry` with `argument` in `RDI`. Its entry
function must terminate through `SYS_THREAD_EXIT`; returning directly is not
supported yet.

`SYS_THREAD_JOIN` blocks only the calling thread. When the sibling target exits,
the scheduler marks it reaped, wakes the joiner, and resumes the saved syscall
context with return value `0`. Self-join, cross-process join, repeated join, and
the simplest two-thread join cycle are rejected.

`SYS_THREAD_SPAWN` is the preferred thread-creation interface. The kernel
reserves and clears a private 16 KiB stack slot from the process page table,
builds the initial context, and reuses the slot after the thread is joined.
`SYS_THREAD_CREATE` remains available for ABI compatibility with callers that
provide their own stack.

`SYS_PIPE` writes two `u64` descriptors to the caller-provided array:
`fds[0]` is read-only and `fds[1]` is write-only. The pipe owns a bounded
256-byte FIFO buffer shared by threads in the creating process. Empty reads
return `ERR_WOULD_BLOCK` while a writer remains open and return `0` (EOF) after
the final writer closes. A full pipe returns `ERR_WOULD_BLOCK`; writes may be
partial when only part of the buffer is available. Cross-process descriptor
inheritance is not implemented yet.

Events are process-owned auto-reset synchronization objects. `SYS_EVENT_WAIT`
consumes a previously latched signal immediately; otherwise it saves the
calling thread's syscall context, marks only that thread Blocked, and appends
its TID to the event's bounded FIFO wait queue. `SYS_EVENT_SIGNAL` wakes the
oldest valid waiter or stores one latched signal when the queue is empty.
Closing an event with active waiters returns `ERR_WOULD_BLOCK`.

Message queues are process-owned bounded FIFO objects. The placeholder supports
8 queue slots system-wide, 8 queued messages per object, and messages up to 64
bytes. Empty receive and full send return `ERR_WOULD_BLOCK`. Message boundaries
are preserved: receive with an undersized user buffer returns
`ERR_INVALID_ARGUMENT` and leaves the message queued. Blocking send/receive and
cross-process handle transfer are future extensions.

`SYS_KILL` currently supports `SIGTERM` (`15`). An unblocked signal terminates
the target user process with status `128 + signal` (`143`), removes all target
threads from scheduling, closes process-owned descriptors and IPC objects, and
wakes a parent blocked in `waitpid`. A blocked `SIGTERM` is stored in the
process pending set and `SYS_KILL` returns `1`; immediate delivery returns `0`.
Root may signal any user process; non-root callers may signal only processes
with the same UID. Kernel tasks, self-targets, already-exited targets, and
unsupported signals are rejected.

`SYS_SIGPROCMASK` accepts `SIG_BLOCK` (`0`), `SIG_UNBLOCK` (`1`), or
`SIG_SETMASK` (`2`) and returns the previous process-wide mask. Signal bit
`signal - 1` represents each signal, so `SIGTERM_MASK` is `1 << 14`.
`SYS_SIGPENDING` returns the process-wide pending set. Unblocking a pending
`SIGTERM` applies its configured disposition immediately.

`SYS_SIGACTION` configures `SIG_DFL` (`0`), `SIG_IGN` (`1`), or an executable
user handler entry. Initial handler entries use the ABI
`extern "C" fn(signal: u64) -> !`: the signal number is passed in `RDI`, and the
handler completes with `SYS_SIGRETURN`. The kernel stores the interrupted
register context in protected thread metadata, schedules the handler, then
restores the original context on signal return. Nested handler delivery is
deferred into the pending set. Process groups and terminal-generated signals
remain future work.

Every user process has a process-group ID (`PGID`) and session ID (`SID`).
The first user process below the kernel becomes its own session and process
group leader; descendants inherit both identities. `SYS_SETPGID` may update the
caller or one of its children within the same session. A new group uses the
target PID as PGID, while joining another group requires that group to exist in
the same session. Session leaders cannot change process groups.
`SYS_SETSID` creates a new session and process group whose IDs equal the caller
PID, and rejects callers that are already process-group leaders.

The terminal converts `Ctrl-C` into `SIGINT` (`2`) and `Ctrl-Z` into
`SIGTSTP` (`20`) for the foreground job. Default `SIGINT` terminates the job
with status `130`. Default `SIGTSTP` preserves its saved context in the new
`Stopped` state, wakes the waiting shell with status `148`, and removes terminal
foreground ownership. `kill <pid> 18` sends `SIGCONT` and makes a stopped
process runnable again. Keyboard IRQ handling only queues the control event;
process lifecycle changes occur later in normal kernel runtime context.

## Stable Error Values

| Value | Name | Meaning |
| --- | --- | --- |
| -1 | `ERR_UNKNOWN_SYSCALL` | Syscall number is not recognized |
| -2 | `ERR_INVALID_ARGUMENT` | Invalid argument, pointer, length, or state |
| -3 | `ERR_NOT_IMPLEMENTED` | Reserved operation is not implemented |
| -4 | `ERR_NO_SUCH_PROCESS` | Process does not exist |
| -5 | `ERR_NOT_CHILD` | Process exists but is not a child of the caller |
| -6 | `ERR_WOULD_BLOCK` | Operation cannot complete without blocking |
| -7 | `ERR_PERMISSION_DENIED` | Caller lacks permission for the operation |

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
- `3+`: process-owned descriptors returned by `SYS_OPEN` or `SYS_PIPE`

`SYS_READ` on stdin may return `-6` (`WouldBlock`) when no input is available.
Userland shells should yield after repeated `WouldBlock` reads.
Pipe reads and writes use the same non-blocking error until IPC wait queues are
implemented.

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
- `read`, `open`, `close`, `pipe`
- `event_create`, `event_wait`, `event_signal`, `event_close`
- `msgq_create`, `msgq_send`, `msgq_receive`, `msgq_close`
- `sigprocmask`, `sigpending`, `sigaction`, `sigreturn`
- `getpgrp`, `setpgid`, `getsid`, `setsid`
- `exec`, `exec_background`, `waitpid`
- `exit`, `yield_now`
- `sleep_ms`
- `getcwd`, `chdir`
- `uptime_ms`, `whoami`, `listdir`, `read_file`, `stat`, `procs`, `kill`
- `file_stat`
- `program_arg`, `argv`, `first_arg`

New Rust ELF commands should import or share this wrapper instead of defining
their own syscall constants and inline assembly.
