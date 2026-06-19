# User Mode Preparation

Vantara OS boots `/bin/init`, login, and the user shell in Ring 3. User
processes run with private page tables, are preempted by the PIT scheduler, and
return to the kernel through the versioned `int 0x80` syscall ABI.

## GDT/TSS Audit

Current state:

- Kernel code segment is loaded.
- TSS is loaded for double-fault IST.
- User code/data segments are installed for Ring 3.

Current hardening:

- ELF load segments are mapped with page-level permissions derived from
  `PF_R/PF_W/PF_X`.
- Writable user pages and user stacks are non-executable.
- Writable+executable ELF segments are rejected.
- User faults terminate the offending process without panicking the kernel.

## Syscall ABI Plan

Initial syscall entry is interrupt-based:

- Interrupt vector: `0x80`
- Syscall number: `rax`
- Args: `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9`
- Return value: `rax`
- Negative values represent errors.

Reserved syscall numbers:

- `1`: `exit`
- `2`: `write`
- `3`: `uptime`
- `4`: `getuid`
- `5`: `getgid`
- `6`: `whoami`
- `7`: `listdir`
- `8`: `read_file`
- `9`: `stat`

## Ring-3 Prototype

Milestone 10.1 adds the first Ring-3 building blocks:

- GDT user code and data descriptors
- IDT syscall gate at `int 0x80` with Ring-3 privilege
- A tiny hand-written user stub image: `SYS_UPTIME(buf); SYS_LISTDIR("/", buf); SYS_EXIT`
- user-accessible code and stack page mapping helpers
- an `iretq` transition helper
- a controlled `userboot` shell command

The normal boot path queues `/bin/init`, which launches login and `/bin/sh`.
The kernel shell remains available only as a development and diagnostic
surface.

The first user task currently exits through `SYS_EXIT`. The syscall marks the
process as exited, re-enables interrupts, reprints the shell prompt, and enters
the shared kernel event loop again. This is an early dispatcher path, not a full
scheduler-backed user process return yet.

`SYS_WRITE` is implemented for the first prototype user page. It validates
stdout/stderr, bounds the length, checks that the pointer stays inside the mapped
prototype user page, then prints the buffer to serial and VGA.

The first Linux-like compatibility layer is intentionally small:

- `SYS_UPTIME` returns milliseconds since boot, or writes decimal uptime text to a user buffer when `arg0/arg1` point to an output buffer.
- `SYS_GETUID` and `SYS_GETGID` return `0`.
- `SYS_WHOAMI` copies `root` into a user buffer.
- `SYS_LISTDIR` writes newline-separated RAM FS entries into a user buffer.
- `SYS_READ_FILE` copies a RAM FS file into a user buffer.
- `SYS_STAT` copies a compact `UserFileStat` record into a user buffer.

## Process Model

Each process has:

- PID
- optional parent PID
- state
- address-space descriptor
- file handle vector placeholder

Each process owns a thread store with one main thread. Scheduling state, saved
Ring-3 context, runtime ticks, and context-switch counters belong to threads
rather than processes. PIDs and TIDs use separate allocators, and the ready
queue stores TIDs. The model can register additional threads sharing a process
address space. ABI v1.1 provides `SYS_THREAD_CREATE` and `SYS_THREAD_EXIT`;
userland supplies a dedicated writable stack buffer, while the kernel validates
non-overlap, constructs the initial register context, assigns a TID, and queues
the thread for scheduling.

The current process table creates a placeholder init process and registers a
first Ring-3 task prototype so kernel-side tools can inspect process state
before Ring-3 execution is enabled.

## ELF Loader Prototype

The current ELF parser validates 64-bit little-endian x86_64 executable headers
and extracts:

- entry point
- program header table offset
- program header count

Segment loading and page mapping are intentionally left for the user-mode boot
work.

## Fault Containment

Ring-3 page faults, divide errors, invalid opcodes, and general-protection faults
terminate only the offending process. The kernel closes its descriptors,
releases its private address-space slot, records a fault-specific exit status,
wakes a waiting parent, and continues scheduling. The same exceptions
originating in Ring 0 remain fatal kernel errors and deliberately panic.
