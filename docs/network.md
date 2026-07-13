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

`network-rx-test` connects two e1000 devices to an isolated QEMU virtual hub.
The second NIC's broadcast reaches the first RX ring, where the kernel validates
the EtherType and marker, records `rx_packets=1`, clears the descriptor, and
advances `RDT` to return ownership to the controller.

The Ethernet II layer now provides bounded frame encoding and parsing. It
extracts destination/source MAC addresses, EtherType, and payload, rejects
short frames and oversized TX payloads, and classifies ARP (`0x0806`), IPv4
(`0x0800`), Vantara-test (`0x88b5`), and unknown protocols. The two-NIC DMA
regression proves the live frame reaches the `vantara-test` dispatcher.

The ARP layer validates Ethernet/IPv4 ARP headers, encodes requests and replies,
learns sender IPv4-to-MAC mappings in a bounded per-interface cache, and answers
requests for the interface's own address. In the isolated two-NIC regression,
`10.0.2.15` broadcasts a request for `10.0.2.16`; the peer learns the sender,
returns a unicast reply, and the requester completes address resolution. The
result and counters are visible through `/Devices/net`.

The IPv4 layer now encodes and parses the minimum header, generates and verifies
the Internet checksum, validates version/IHL/total length and destination, and
rejects malformed or fragmented packets. After ARP resolution, the regression
sends a protocol-253 packet from `10.0.2.15` to `10.0.2.16` and validates its
source, destination, protocol, checksum, and payload after RX DMA.

UDP now encodes and parses source/destination ports, validates datagram length,
and generates and verifies the mandatory test checksum over the IPv4
pseudo-header. The live regression delivers `VANTARA_UDPV1` from port `40000`
to port `7777` after ARP resolution and validates it after RX DMA.

The kernel UDP socket layer provides bounded `bind`, non-blocking `receive`, and
`close` operations. Destination-port dispatch places validated datagrams into a
four-entry socket queue, rejects duplicate binds, and counts controlled drops
when a queue is full. The QEMU path now proves the packet reaches a socket bound
to port `7777`, rather than stopping at the UDP parser.

UDP also has a kernel send API. A caller sends through a bound socket handle, so
the source port comes from the socket table. The TX path validates destination
ports and payload size, resolves the destination through the per-interface ARP
cache, then builds UDP, IPv4, and Ethernet headers before submitting the frame to
the e1000 transmit ring. The live QEMU regression sends from a socket bound to
port `40000` and receives on the peer socket bound to `7777`.

The syscall ABI now exposes UDP `bind`, `send_to`, `recv_from`, and `close`
operations to userland. `/bin/udpdemo` validates the public ABI surface,
including duplicate-bind rejection, non-blocking empty receive semantics, and
clean close. UDP send from userland succeeds when a route is already resolved,
and otherwise reports a non-blocking unavailable state.

The scheduler-owned kernel event thread now polls completed RX descriptors on
every runtime wake-up. Each pass is bounded to one descriptor-ring traversal
per device and never spin-waits for a packet. This lets datagrams that arrive
after boot reach a user socket while normal processes are running.

The TCP wire-format foundation now encodes and parses minimum TCP headers,
sequence and acknowledgment numbers, control flags, receive windows, payloads,
and the IPv4 pseudo-header checksum. Malformed header lengths, reserved port
zero, oversized payloads, and invalid checksums are rejected. A bounded passive
connection table validates the `LISTEN -> SYN-RECEIVED -> ESTABLISHED` state
transitions and sequence/acknowledgment numbers. Wiring those actions to live
two-NIC frame transmission is now complete: the QEMU regression sends a SYN
from `10.0.2.15:40001` to `10.0.2.16:8080`, validates the returned SYN-ACK,
sends the final ACK, and requires both interfaces to report an established
connection.

The established path now accepts in-order payload, advances the peer sequence,
and returns a cumulative ACK. The same live regression transfers `TCPDAT`, then
sends FIN and verifies the passive endpoint acknowledges the consumed FIN and
moves the bounded connection entry to closed state.

The kernel now exposes bounded TCP socket handles for passive `listen`,
non-blocking `accept` and `receive`, established-stream `send`, and `close`.
Accepted connections own a four-entry receive queue, and the live boot path
consumes `TCPDAT` through this socket API. Active `connect` now allocates an
accepted client handle, transmits SYN through the resolved route, validates the
SYN-ACK sequence, emits the final ACK, and transitions from `SYN-SENT` to
`ESTABLISHED`.

ABI v1.14 exposes TCP `listen`, non-blocking `accept`, active `connect`, `send`,
non-blocking `receive`, and `close` to native userland. `/bin/tcpdemo` creates a
listener on port 8081, connects through the peer NIC from port 40200, accepts
the connection, transfers `TCPUSR`, validates the received bytes, and closes
all three socket handles in the two-NIC command smoke regression.
