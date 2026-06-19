# Kernel Synchronization

This document records the current lock ordering and interrupt-safety rules for
Vantara OS.

## Rules

- Keep interrupt handlers short.
- Do not print from hot interrupt paths.
- Use atomics for timer/IRQ counters when possible.
- Use `sync::without_interrupts` around VGA and serial writes.
- Timer preemption currently touches the process table and ready queue. These
  structures must not allocate while the timer IRQ is active.
- Prefer acknowledging PIC IRQs before driver processing for keyboard and mouse.

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

- `input`: may be touched from keyboard and mouse IRQ handlers; avoid logging
  while holding `INPUT_QUEUE`.
- `vga`: screen writes are wrapped with interrupts disabled to avoid recursive
  lock attempts from interrupt context.
- `scheduler`: the timer IRQ can transition the current user thread to Ready,
  select another pre-existing ready thread, and switch CR3. It must never grow
  a vector, print, or take a lock that can already be held by interrupted code.
- `allocator`: heap allocation must not happen before `allocator::init_heap`.
  Avoid allocation in IRQ handlers.
