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

Submitting and reclaiming the first Ethernet frame is the next NIC milestone;
Ethernet parsing, ARP, IPv4, UDP, and TCP remain pending.
