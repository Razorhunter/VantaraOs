# Storage Stack

The default VANTFS persistence stack is:

```text
storage-auto (NVMe preferred, then AHCI, then ATA PIO)
    -> MBR partition view -> write-through LRU cache -> VANTFS
```

Backend selection is compile-time and explicit:

- default builds enable `storage-auto`;
- `storage-nvme` strictly requires the primary 512-byte-LBA NVMe namespace;
- `storage-ahci` strictly requires the second discovered AHCI SATA disk;
- `storage-ata` strictly requires the primary-slave ATA PIO disk;
- `storage-auto` prefers NVMe, then falls back to the second AHCI disk and
  finally ATA PIO;
- strict and automatic policy features are mutually exclusive.

Selection emits the policy, preferred backend, selected backend, and whether a
fallback occurred. `storage-policy-test` proves all three auto-policy branches:
NVMe and AHCI persistence on Q35 plus ATA PIO persistence on a PC/IDE machine.

PCI/AHCI initialization happens before private user page tables are prepared,
so the audited MMIO window is present when filesystem syscalls execute in a
user process address space.

## AHCI Foundation

Vantara now discovers PCI AHCI controllers using class `01/06/01` and decodes
BAR5 as the AHCI Base Address Register (ABAR). Detected controllers are recorded
in the driver registry and exposed through:

```text
/dev/ahci
```

The kernel maps each ABAR into a reserved high-half MMIO window. Mappings are
page-aligned, uncached, write-through, writable for device registers, and
non-executable. It performs non-destructive volatile reads of HBA capability,
global-host-control, implemented-port, and version registers. These values are
shown in `/dev/ahci`.

For the first active SATA link (`DET=3`, `IPM=1`, SATA signature), the kernel
now stops the port command engine, allocates exclusive page-aligned DMA frames,
and installs:

- a 1 KiB command list;
- a 256-byte received-FIS area;
- a slot-0 command table;
- one PRDT entry backed by a 4 KiB data frame.

It clears stale port status, disables port interrupts, and restarts the engine
with timeout guards around `PxCMD.CR` and `PxCMD.FR`. The driver then builds a
Register Host-to-Device FIS, issues `IDENTIFY DEVICE` (`0xec`) through slot 0,
polls `PxCI` with task-file error and busy/DRQ guards, and parses the ATA
word-swapped model string. It reuses the guarded slot executor for a
single-sector `READ DMA EXT` (`0x25`) of LBA 0 and verifies the boot signature
`55aa`. Both results are visible in `/dev/ahci`.

The driver exposes this path as an `AhciBlockDevice`. Its capacity comes from
the IDENTIFY LBA48 sector-count words, reads are range checked, and the AHCI
registry lock serializes access to the shared slot-0 DMA structures. A boot-time
probe reads LBA 0 through the `BlockDevice` trait.

Writes use `WRITE DMA EXT` (`0x35`) with the command-header write bit and a
512-byte host-to-device DMA payload. Every successful `write_block` immediately
issues `FLUSH CACHE EXT` (`0xea`) before returning, preserving synchronous
write-through semantics. A feature-gated regression appends safe scratch space
to a copied boot image, writes its final sector, and proves the marker survives
a second QEMU boot. Normal kernel builds never run this destructive probe.

The tested automatic policy is now the default. Strict ATA and AHCI builds
remain available for diagnosis, recovery, and backend-specific regression.

That isolated integration is now covered by the `ahci-vantfs-test` feature. On
a copied Q35 boot disk, the kernel creates a type-`0x7f` MBR partition in
appended scratch space, wraps `AhciBlockDevice` in `PartitionBlockDevice` and
the 16-sector write-through cache, formats VANTFS, and writes a file. A second
boot reparses the MBR and verifies the same payload through the full stack.
Normal builds still select ATA PIO, so these proofs do not silently change the
user's `/persist` backend. A separate `ahci-persist-test` feature build runs the
actual shell lifecycle (`mkdir`, `write`, reboot, `cat`) against a second Q35
disk and verifies that the selected backend is `ahci` on both boots.

The AHCI persistence migration is complete.

## NVMe Foundation

Vantara detects PCI NVMe controllers using class `01/08/02`, validates BAR0 as
a memory BAR, and maps a dedicated 16 KiB uncached, write-through,
non-executable MMIO window. Discovery reads:

- controller capabilities and version (`CAP`, `VS`);
- controller configuration and status (`CC`, `CSTS`);
- existing admin queue attributes and addresses (`AQA`, `ASQ`, `ACQ`).

It derives the maximum queue entries, doorbell stride, and supported memory
page-size range. The driver then clears `CC.EN`, polls `CSTS.RDY` with fatal and
timeout guards, allocates exclusive page-aligned admin submission/completion
queue frames, and programs a 64-entry `AQA/ASQ/ACQ`. It enables the controller
with 64-byte submission and 16-byte completion entries and waits for ready.

Controllers and private queue state are exposed through `/dev/nvme`.
`nvme-test` boots QEMU with an isolated NVMe data image and copied kernel image,
then verifies ownership and queue setup without conflicting with a concurrently
running development QEMU instance.

The kernel submits polled `Identify Controller` and `Identify Namespace`
commands through admin queue 0. Submission/completion doorbells use the CAP
doorbell stride; completion command ID, queue ID, status, and phase are checked
before the 4 KiB DMA result is parsed. `/dev/nvme` now reports model, serial,
controller namespace count, NSID 1 size/capacity, and active LBA size.

The driver requests one I/O queue pair with Set Features, creates a physically
contiguous 64-entry completion queue followed by its linked submission queue at
QID 1, and uses the same phase/status-checked polling path for NVM commands. A
single-block read of NSID 1 LBA 0 is verified against the `VANTNVME` marker in
the isolated test image.

The NVM write path copies one 512-byte sector into the queue DMA buffer, submits
Write, checks its completion, and follows it with an explicit Flush command.
`nvme-write-test` writes the final namespace LBA, validates immediate readback,
then boots the same image again and compares checksums to prove persistence.

The synchronized `NvmeBlockDevice` exposes the 512-byte namespace through the
common block interface. Reads and writes are serialized by the controller
registry lock, bounds checked against NSZE, and every successful write is
followed by NVM Flush.

`storage-auto` now selects NVMe first and the two-boot policy regression proves
the complete NVMe -> superfloppy -> cache -> VANTFS `/persist` lifecycle. The
runtime queue API copies into caller-provided buffers, keeping 4 KiB Identify
payloads out of the smaller syscall stack while block reads transfer 512 bytes.
