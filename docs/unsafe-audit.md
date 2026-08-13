# Unsafe Rust Audit

Vantara keeps unsafe Rust at explicit hardware, memory-management, and
privilege-transition boundaries. Safe kernel code must reach these operations
through validated APIs rather than dereferencing arbitrary addresses directly.

## Required Invariants

| Boundary | Files | Safety invariant |
| --- | --- | --- |
| Boot memory and paging | `memory.rs`, `user/address_space.rs`, `allocator.rs` | Bootloader physical-memory offset is valid; page-table frames are live and uniquely mutated; mappings use allocated frames and canonical addresses. |
| Ring-3 transitions | `user/ring3.rs`, `runtime.rs`, `scheduler/*` | CR3 belongs to the selected process, kernel mappings remain present, GDT selectors are installed, saved RIP/RSP/register state belongs to that task, and ELF pages obey W^X. |
| Boot handoff | `main.rs`, `memory.rs`, `drivers/framebuffer.rs` | BootInfo memory regions remain static, only `Usable` frames enter the allocator, the physical-memory offset is present, and framebuffer address/stride/size stay mapped for kernel ownership. |
| User pointers | `user/syscall.rs` | Every complete range belongs to the current process and each page is present, user-accessible, and writable when the kernel writes. |
| ELF loading | `user/elf.rs`, `user/loader.rs` | ELF headers and segment bounds are validated before copying; destinations stay inside prepared user mappings. |
| Allocators | `allocator/*` | Heap range is mapped once, aligned, exclusively allocator-owned, and free-list nodes describe valid non-overlapping regions. |
| Interrupts and CPU tables | `interrupts.rs`, `gdt.rs` | IDT/GDT/TSS live for the kernel lifetime; custom entry stubs preserve the documented register/CPU frame layout; handler signatures match CPU frames; PIC access uses configured ports and offsets. |
| Device I/O | `drivers/*`, `serial.rs`, `timer.rs`, `power.rs`, `vga_buffer.rs` | Fixed ports/MMIO addresses match the selected x86 platform and access is serialized by the owning driver or lock. VirtIO capability bounds and register alignment are validated before volatile MMIO, and queue frames remain device-owned after enable. |

## Review Rules

- Every new unsafe operation needs a local `SAFETY:` note or must be covered by
  a documented module boundary above.
- Public unsafe functions must state caller obligations in nearby rustdoc.
- Unsafe code must validate lengths and checked arithmetic before constructing
  slices or copying bytes.
- Ring-0 faults remain fatal; Ring-3 faults must go through process containment.
- `unsafe_op_in_unsafe_fn` is denied, so unsafe functions still require explicit
  blocks around each operation.
- Changes to the unsafe baseline require updating
  `scripts/unsafe-baseline.tsv` during review.

Run the audit gate with:

```bash
make unsafe-audit
```
