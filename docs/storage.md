# Storage Stack

The active VANTFS persistence stack is:

```text
ATA PIO -> MBR partition view -> write-through LRU cache -> VANTFS
```

## AHCI Foundation

Vantara now discovers PCI AHCI controllers using class `01/06/01` and decodes
BAR5 as the AHCI Base Address Register (ABAR). Detected controllers are recorded
in the driver registry and exposed through:

```text
/dev/ahci
```

The current state is discovery-only. The kernel does not dereference ABAR or
start the AHCI command engine yet because PCI MMIO ranges need an explicit,
audited mapper rather than assuming they are covered by the bootloader physical
memory mapping.

The next AHCI steps are:

1. map and validate the ABAR MMIO range;
2. read HBA capability/version/implemented-port registers;
3. allocate aligned command-list, received-FIS, command-table, and PRDT memory;
4. implement polled IDENTIFY and single-sector DMA read;
5. add write/flush support and only then consider replacing ATA PIO.

NVMe remains a later sibling driver behind the same `BlockDevice` boundary.
