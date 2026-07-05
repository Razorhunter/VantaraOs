# Network Stack

Vantara currently provides the first NIC-driver foundation for Intel e1000
hardware. PCI discovery recognizes the QEMU 82540EM (`8086:100e`) and 82574L
e1000e (`8086:10d3`) devices.

The kernel decodes memory BAR0, maps a dedicated uncached MMIO window before
private user address spaces are prepared, and reads the controller `CTRL`,
`STATUS`, link state, and receive-address registers. A valid MAC address and
the live controller state are exposed through:

```text
/Devices/net
```

The compatibility path `/dev/net` resolves to the same devfs snapshot. RX/TX
DMA is now enabled through the PCI command register. The driver allocates
page-aligned 16-entry receive and transmit descriptor rings, gives every
descriptor a dedicated packet-buffer frame, programs ring base/length/head/tail
registers, and enables receive/transmit engines with interrupts masked for the
current polling foundation.

The boot-time TX regression builds a minimum 60-byte broadcast Ethernet frame,
uses the NIC MAC as source, assigns experimental EtherType `0x88b5`, and writes
the `VANTARA_TX_V1` marker. It advances `TDT`, polls descriptor-done, reclaims
the slot, and exposes `tx_test=1` plus `tx_packets=1` through `/Devices/net`.

Polling, validating, and recycling received descriptors is the next NIC
milestone; Ethernet parsing, ARP, IPv4, UDP, and TCP remain pending.
