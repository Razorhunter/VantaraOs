# Scheduler Model

Vantara uses one global TID namespace for kernel and user threads.

- The process table owns thread identity, lifecycle state, context-switch
  counters, and runtime accounting.
- The kernel scheduler owns kernel stacks and saved kernel CPU contexts.
- The user scheduler stores Ring-3 contexts directly in the owning thread.
- Kernel scheduler ready-queue entries and `current_task` values are TIDs, not
  a separate task-ID namespace.
- Idle and demo kernel threads belong to the kernel process.

## Ring-0 timer switching

Normal boot now registers the runtime event loop as a kernel thread after
drivers, filesystems, and the initial user-program request are prepared.
`start_first_task` then leaves the temporary boot stack and starts that runtime
thread on its owned scheduler stack. The idle thread remains available when no
normal kernel work is runnable.

The `kernel-thread-preemption-test` feature replaces the normal runtime
workload with a controlled live timer-driven test between kernel threads:

- A fresh thread enters through its synthetic stack frame and entry point.
- A preempted thread resumes through the exact CPU interrupt frame saved by the
  PIT entry stub.
- The QEMU regression starts three CPU-bound threads and requires the first two
  to make new progress after the third thread starts. This verifies both initial
  dispatch and resume of previously preempted Ring-0 contexts.

Normal boot has live Ring-0 scheduling enabled. Scheduler-critical sections and
kernel mutexes have explicit preemption-disable coverage through
`PreemptionGuard` and `PreemptMutex`.

The QEMU test keeps timer IRQs active inside a nested preemption-disabled
critical section and verifies that no other kernel thread runs until the guard
is released. This model is currently single-core; SMP requires per-CPU
preemption state.
