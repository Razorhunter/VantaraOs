# Kernel Synchronization

This document records the current lock ordering and interrupt-safety rules for
Vantara OS.

## Rules

- Keep interrupt handlers short.
- Do not print from hot interrupt paths.
- Use atomics for timer/IRQ counters when possible.
- Kernel-owned mutexes use `sync::PreemptMutex`. Its guard disables local
  interrupts and increments the preemption-disable nesting depth before taking
  the underlying spin lock.
- Use `sync::PreemptionGuard` for lock-free critical sections that may keep
  timer interrupts enabled but must not be involuntarily rescheduled.
- Use `sync::without_interrupts` around compound IRQ-sensitive operations that
  do not acquire a mutex.
- Timer preemption currently touches the process table and ready queue. These
  structures must not allocate while the timer IRQ is active.
- Prefer acknowledging PIC IRQs before driver processing for keyboard and mouse.

The nesting counter is currently global because Vantara boots one CPU. SMP work
must replace it with per-CPU state before additional processors are enabled.

## Deferred Ring-0 preemption

If a PIT interrupt arrives while `PreemptionGuard` is active, the scheduler:

1. records that a preemption was deferred;
2. acknowledges the timer without switching kernel stacks;
3. permits a later timer interrupt to switch after the outermost guard drops.

`PreemptMutex` also disables local interrupts, so the PIT normally cannot enter
while that mutex is held. The preemption counter remains necessary for nested
guards and lock-free scheduler-critical sections.

## Lock Ordering

When multiple locks are needed, acquire them in this order:

1. `allocator`
2. `scheduler`
3. `input`
4. `vga`
5. `serial`

Avoid holding any lock while calling into another subsystem unless the ordering
above is respected. In particular, input handlers should enqueue events and let
the main loop or shell perform VGA/serial output later.

## Subsystem Notes

- `input`: may be touched from keyboard and mouse IRQ handlers; `PreemptMutex`
  prevents local IRQ re-entry while `INPUT_QUEUE` is held. Avoid logging while
  holding it.
- `vga`: the writer uses `PreemptMutex`; compound rendering operations may also
  use `without_interrupts`.
- `scheduler`: the timer IRQ can transition the current user thread to Ready,
  select another pre-existing ready thread, and switch CR3. It must never grow
  a vector, print, or take a lock that can already be held by interrupted code.
- `allocator`: its lock is preemption- and IRQ-safe. Heap allocation must not
  happen before `allocator::init_heap`, and allocation remains forbidden in IRQ
  handlers.
