# Vantara OS

Vantara OS is an experimental x86_64 kernel written in Rust. The current kernel
boots through `bootloader`, initializes GDT/IDT/PIC, sets up paging and heap
allocation, handles basic PS/2 keyboard and mouse input, and contains an early
scheduler prototype.

## Requirements

- Docker with Compose support
- QEMU, if running directly on the host
- Rust nightly with `rust-src`, `llvm-tools-preview`, and `bootimage`, if
  building directly on the host

The Docker builder is the recommended path because it keeps the kernel toolchain
isolated.

## Build With Docker

```bash
make build
make shell
make kernel-build
```

Inside the Docker shell, the project is mounted at `/workspace`.
QEMU is installed inside the builder image, so host installation is optional.

Run the timer-preemption integration test without installing QEMU on the host:

```bash
make build
make docker-preemption-test
```

Run the complete regression suite in Docker:

```bash
make docker-regression
```

## Build On Host

```bash
make kernel-fmt
make kernel-check
make kernel-build
```

The boot image is generated at:

```text
kernel/target/x86_64-vantara_os/debug/bootimage-kernel.bin
```

## Run

```bash
make run
```

Every normal run attaches `target/vantara-persist.img` as the second disk.
The image is created once and reused, so files under `/persist` survive QEMU
restarts. QEMU exposes its display through VNC at `localhost:5900`.

Run through Docker and connect a VNC client to `localhost:5901`:

```bash
make docker-run
```

The Docker target uses VNC display `:1` by default to avoid colliding with a
host-side `make run` session on port 5900.

To deliberately erase persistent filesystem data:

```bash
make reset-persist
```

## Test

```bash
make kernel-check
make kernel-test
make regression
make unsafe-audit
make smoke
make boot-test
make command-smoke
make preemption-test
make isolation-test
make artifact-manifest
make package-test
make block-cache-test
make partition-test
```

`kernel-check` performs a compile check for the kernel and integration tests.
`kernel-test` compile-checks the in-kernel unit and integration test targets.
`regression` verifies category coverage, then runs the executable process,
memory, syscall, filesystem, and boot QEMU quality gate used by local CI.
`unsafe-audit` checks the reviewed unsafe-boundary baseline and required safety
invariants documented in [`docs/unsafe-audit.md`](docs/unsafe-audit.md).
`smoke` runs a headless QEMU boot and checks serial output for core boot markers.
`boot-test` verifies the automatic `/bin/init -> login -> sh` path.
`service-manager-test` verifies that the real PID 1 supervises `/bin/login` and
restarts it after termination.
`device-namespace-test` verifies the `/dev` mount, null/zero device semantics,
and live driver, PCI, and network registry views.
`block-cache-test` verifies non-zero VANTFS cache hits, misses, and write-through
operations in QEMU.
`partition-test` verifies MBR discovery, partition-relative VANTFS I/O, and
data persistence across reboot.
`command-smoke` boots an isolated QEMU guest for each of `ls`, `cat`, `procs`,
and `rusthello`, then compares its serial output with stable golden snippets.
`preemption-test` proves that a CPU-bound background process which never calls
`yield` is involuntarily descheduled while the shell remains runnable. Use
`docker-preemption-test` when QEMU is not installed on the host.
`isolation-test` triggers null and stack-guard faults in user processes, verifies
that only the offending process dies, then runs `rusthello` to prove recovery.
`artifact-manifest` writes `target/artifact-manifest.tsv` with the relative path,
byte size, and SHA-256 digest of the boot image, generated userland registry,
and canonical userland artifacts.
`package-test` builds and verifies `target/vantara-dev.tar.gz`, containing the
kernel boot image, canonical initrd, blank persistence disk, and integrity
metadata. See [`docs/packaging.md`](docs/packaging.md).
`kernel-build` refreshes the source artifact manifest automatically. The boot
banner reports version, profile, Git
commit/dirty state, and build timestamp. Set `SOURCE_DATE_EPOCH` for a
reproducible UTC timestamp, or override with `VANTARA_BUILD_TIMESTAMP`,
`VANTARA_GIT_COMMIT`, and `VANTARA_GIT_DIRTY`.

Before sharing a development image, complete
[`docs/release-checklist.md`](docs/release-checklist.md).

## Current Kernel Features

- x86_64 custom target
- VGA text output
- serial output
- GDT, IDT, PIC initialization
- timer tick counter at 100 Hz
- IRQ enable/disable and PIC acknowledge helpers
- keyboard and mouse event counters
- keyboard raw key, typed char, release, and modifier-state events
- PS/2 mouse initialization and VGA input debug overlay
- VGA console API and command shell
- scheduler task lifecycle, idle task, cooperative yield, and tick stats
- synchronization helpers and lock-ordering notes
- PCI discovery and UHCI lookup through PCI BARs
- PS/2 controller initialization
- read-only RAM filesystem with `ls`, `cat`, and `stat`
- nested writable `/tmp` RAM filesystem and persistent `/persist` VANTFS volume
- user-mode preparation: syscall ABI, process model, ELF parser prototype
- reserved write filesystem shell commands: `cp`, `mv`, `rm`
- boot banner and serial boot self-check summary
- serial log macros for error/warn/info/debug
- VGA panic screen
- QEMU shutdown and keyboard-controller reboot helpers
- paging and heap allocation
- frame allocator and heap diagnostics
- reusable page map, unmap, translate, and range mapping helpers
- PS/2 keyboard and mouse input queue
- early task scheduler structures
- bootimage/QEMU test support
