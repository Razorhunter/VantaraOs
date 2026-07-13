use crate::sync::PreemptMutex as Mutex;
use alloc::vec::Vec;
use core::sync::atomic::{Ordering, fence};
use lazy_static::lazy_static;
use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{FrameAllocator, Mapper, Size4KiB},
};

use crate::drivers::pci::{self, PciBar, PciDevice};

const PCI_CLASS_NETWORK: u8 = 0x02;
const PCI_SUBCLASS_ETHERNET: u8 = 0x00;
const INTEL_VENDOR_ID: u16 = 0x8086;
const INTEL_E1000_DEVICE_ID: u16 = 0x100e;
const INTEL_E1000E_DEVICE_ID: u16 = 0x10d3;
const E1000_MMIO_WINDOW: u64 = 0xffff_9200_0000_0000;
const E1000_MMIO_STRIDE: u64 = 0x20_000;
const E1000_MMIO_SIZE: u64 = 0x20_000;
const REG_CTRL: u64 = 0x0000;
const REG_STATUS: u64 = 0x0008;
const REG_RECEIVE_ADDRESS_LOW: u64 = 0x5400;
const REG_RECEIVE_ADDRESS_HIGH: u64 = 0x5404;
const STATUS_LINK_UP: u32 = 1 << 1;
const RECEIVE_ADDRESS_VALID: u32 = 1 << 31;
const REG_INTERRUPT_MASK_CLEAR: u64 = 0x00d8;
const REG_RECEIVE_CONTROL: u64 = 0x0100;
const REG_TRANSMIT_CONTROL: u64 = 0x0400;
const REG_TRANSMIT_IPG: u64 = 0x0410;
const REG_RECEIVE_DESC_LOW: u64 = 0x2800;
const REG_RECEIVE_DESC_HIGH: u64 = 0x2804;
const REG_RECEIVE_DESC_LENGTH: u64 = 0x2808;
const REG_RECEIVE_DESC_HEAD: u64 = 0x2810;
const REG_RECEIVE_DESC_TAIL: u64 = 0x2818;
const REG_TRANSMIT_DESC_LOW: u64 = 0x3800;
const REG_TRANSMIT_DESC_HIGH: u64 = 0x3804;
const REG_TRANSMIT_DESC_LENGTH: u64 = 0x3808;
const REG_TRANSMIT_DESC_HEAD: u64 = 0x3810;
const REG_TRANSMIT_DESC_TAIL: u64 = 0x3818;
const RECEIVE_ENABLE: u32 = 1 << 1;
const RECEIVE_BROADCAST_ACCEPT: u32 = 1 << 15;
const RECEIVE_STRIP_CRC: u32 = 1 << 26;
const TRANSMIT_ENABLE: u32 = 1 << 1;
const TRANSMIT_PAD_SHORT_PACKETS: u32 = 1 << 3;
const DMA_RING_DEPTH: usize = 16;
const ETHERNET_MIN_FRAME_SIZE: usize = 60;
const TRANSMIT_COMMAND_EOP_IFCS_RS: u8 = (1 << 0) | (1 << 1) | (1 << 3);
const DESCRIPTOR_DONE: u8 = 1;
const TRANSMIT_POLL_LIMIT: usize = 2_000_000;
const ETHERNET_HEADER_SIZE: usize = 14;
const ETHER_TYPE_IPV4: u16 = 0x0800;
const ETHER_TYPE_ARP: u16 = 0x0806;
const ETHER_TYPE_VANTARA_TEST: u16 = 0x88b5;
const TEST_PAYLOAD: &[u8; 13] = b"VANTARA_TX_V1";
const ARP_PACKET_SIZE: usize = 28;
const ARP_HARDWARE_ETHERNET: u16 = 1;
const ARP_OPERATION_REQUEST: u16 = 1;
const ARP_OPERATION_REPLY: u16 = 2;
const IPV4_MIN_HEADER_SIZE: usize = 20;
const IPV4_MAX_PACKET_SIZE: usize = ETHERNET_MIN_FRAME_SIZE - ETHERNET_HEADER_SIZE;
const IP_PROTOCOL_UDP: u8 = 17;
const UDP_HEADER_SIZE: usize = 8;
const UDP_MAX_DATAGRAM_SIZE: usize = IPV4_MAX_PACKET_SIZE - IPV4_MIN_HEADER_SIZE;
const UDP_TEST_SOURCE_PORT: u16 = 40000;
const UDP_TEST_DESTINATION_PORT: u16 = 7777;
const UDP_TEST_PAYLOAD: &[u8; 13] = b"VANTARA_UDPV1";
const UDP_MAX_PAYLOAD_SIZE: usize = UDP_MAX_DATAGRAM_SIZE - UDP_HEADER_SIZE;
const UDP_SOCKET_LIMIT: usize = 8;
const UDP_SOCKET_QUEUE_DEPTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdpSocketError {
    InvalidPort,
    AddressInUse,
    SocketLimit,
    InvalidHandle,
    WouldBlock,
    PayloadTooLarge,
    NoRoute,
    ArpUnresolved,
    TransmitFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpSocketHandle(u16);

impl UdpSocketHandle {
    pub fn from_raw(raw: u64) -> Result<Self, UdpSocketError> {
        if raw == 0 || raw > u64::from(u16::MAX) {
            return Err(UdpSocketError::InvalidHandle);
        }
        Ok(Self(raw as u16))
    }

    pub fn as_raw(self) -> u64 {
        u64::from(self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceivedUdpDatagram {
    pub source_ip: [u8; 4],
    pub source_port: u16,
    pub destination_port: u16,
    pub payload: [u8; UDP_MAX_PAYLOAD_SIZE],
    pub payload_len: u8,
}

#[derive(Debug)]
struct UdpSocket {
    handle: UdpSocketHandle,
    local_port: u16,
    queue: Vec<ReceivedUdpDatagram>,
    delivered: u64,
    dropped: u64,
}

#[derive(Debug)]
struct UdpSocketTable {
    sockets: Vec<UdpSocket>,
    next_handle: u16,
}

impl UdpSocketTable {
    fn new() -> Self {
        Self {
            sockets: Vec::new(),
            next_handle: 1,
        }
    }

    fn bind(&mut self, local_port: u16) -> Result<UdpSocketHandle, UdpSocketError> {
        if local_port == 0 {
            return Err(UdpSocketError::InvalidPort);
        }
        if self
            .sockets
            .iter()
            .any(|socket| socket.local_port == local_port)
        {
            return Err(UdpSocketError::AddressInUse);
        }
        if self.sockets.len() >= UDP_SOCKET_LIMIT {
            return Err(UdpSocketError::SocketLimit);
        }
        let handle = UdpSocketHandle(self.next_handle);
        self.next_handle = self.next_handle.wrapping_add(1).max(1);
        self.sockets.push(UdpSocket {
            handle,
            local_port,
            queue: Vec::new(),
            delivered: 0,
            dropped: 0,
        });
        Ok(handle)
    }

    fn deliver(&mut self, datagram: ReceivedUdpDatagram) -> bool {
        let Some(socket) = self
            .sockets
            .iter_mut()
            .find(|socket| socket.local_port == datagram.destination_port)
        else {
            return false;
        };
        if socket.queue.len() >= UDP_SOCKET_QUEUE_DEPTH {
            socket.dropped = socket.dropped.saturating_add(1);
            return false;
        }
        socket.queue.push(datagram);
        socket.delivered = socket.delivered.saturating_add(1);
        true
    }

    fn receive(&mut self, handle: UdpSocketHandle) -> Result<ReceivedUdpDatagram, UdpSocketError> {
        let socket = self
            .sockets
            .iter_mut()
            .find(|socket| socket.handle == handle)
            .ok_or(UdpSocketError::InvalidHandle)?;
        if socket.queue.is_empty() {
            return Err(UdpSocketError::WouldBlock);
        }
        Ok(socket.queue.remove(0))
    }

    fn local_port(&self, handle: UdpSocketHandle) -> Result<u16, UdpSocketError> {
        self.sockets
            .iter()
            .find(|socket| socket.handle == handle)
            .map(|socket| socket.local_port)
            .ok_or(UdpSocketError::InvalidHandle)
    }

    fn close(&mut self, handle: UdpSocketHandle) -> Result<(), UdpSocketError> {
        let index = self
            .sockets
            .iter()
            .position(|socket| socket.handle == handle)
            .ok_or(UdpSocketError::InvalidHandle)?;
        self.sockets.remove(index);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdpError {
    DatagramTooShort,
    InvalidLength,
    InvalidChecksum,
    PayloadTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpDatagram<'a> {
    pub source_port: u16,
    pub destination_port: u16,
    pub payload: &'a [u8],
}

impl<'a> UdpDatagram<'a> {
    pub fn parse(
        source_ip: [u8; 4],
        destination_ip: [u8; 4],
        bytes: &'a [u8],
    ) -> Result<Self, UdpError> {
        if bytes.len() < UDP_HEADER_SIZE {
            return Err(UdpError::DatagramTooShort);
        }
        let length = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
        if length < UDP_HEADER_SIZE || length > bytes.len() {
            return Err(UdpError::InvalidLength);
        }
        let checksum = u16::from_be_bytes([bytes[6], bytes[7]]);
        if checksum == 0 || udp_checksum(source_ip, destination_ip, &bytes[..length]) != 0 {
            return Err(UdpError::InvalidChecksum);
        }
        Ok(Self {
            source_port: u16::from_be_bytes([bytes[0], bytes[1]]),
            destination_port: u16::from_be_bytes([bytes[2], bytes[3]]),
            payload: &bytes[UDP_HEADER_SIZE..length],
        })
    }
}

fn udp_checksum(source_ip: [u8; 4], destination_ip: [u8; 4], datagram: &[u8]) -> u16 {
    let mut bytes = [0u8; 12 + UDP_MAX_DATAGRAM_SIZE];
    bytes[..4].copy_from_slice(&source_ip);
    bytes[4..8].copy_from_slice(&destination_ip);
    bytes[9] = IP_PROTOCOL_UDP;
    bytes[10..12].copy_from_slice(&(datagram.len() as u16).to_be_bytes());
    bytes[12..12 + datagram.len()].copy_from_slice(datagram);
    internet_checksum(&bytes[..12 + datagram.len()])
}

fn encode_udp_datagram(
    out: &mut [u8; UDP_MAX_DATAGRAM_SIZE],
    source_ip: [u8; 4],
    destination_ip: [u8; 4],
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
) -> Result<usize, UdpError> {
    let length = UDP_HEADER_SIZE + payload.len();
    if length > out.len() {
        return Err(UdpError::PayloadTooLarge);
    }
    out.fill(0);
    out[..2].copy_from_slice(&source_port.to_be_bytes());
    out[2..4].copy_from_slice(&destination_port.to_be_bytes());
    out[4..6].copy_from_slice(&(length as u16).to_be_bytes());
    out[UDP_HEADER_SIZE..length].copy_from_slice(payload);
    let checksum = udp_checksum(source_ip, destination_ip, &out[..length]);
    out[6..8].copy_from_slice(&if checksum == 0 { u16::MAX } else { checksum }.to_be_bytes());
    Ok(length)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ipv4Error {
    PacketTooShort,
    InvalidVersion,
    InvalidHeaderLength,
    InvalidTotalLength,
    InvalidChecksum,
    Fragmented,
    PayloadTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Packet<'a> {
    pub source: [u8; 4],
    pub destination: [u8; 4],
    pub protocol: u8,
    pub ttl: u8,
    pub payload: &'a [u8],
}

impl<'a> Ipv4Packet<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Ipv4Error> {
        if bytes.len() < IPV4_MIN_HEADER_SIZE {
            return Err(Ipv4Error::PacketTooShort);
        }
        if bytes[0] >> 4 != 4 {
            return Err(Ipv4Error::InvalidVersion);
        }
        let header_length = usize::from(bytes[0] & 0x0f) * 4;
        if header_length < IPV4_MIN_HEADER_SIZE || header_length > bytes.len() {
            return Err(Ipv4Error::InvalidHeaderLength);
        }
        let total_length = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
        if total_length < header_length || total_length > bytes.len() {
            return Err(Ipv4Error::InvalidTotalLength);
        }
        if internet_checksum(&bytes[..header_length]) != 0 {
            return Err(Ipv4Error::InvalidChecksum);
        }
        let fragment = u16::from_be_bytes([bytes[6], bytes[7]]);
        if fragment & 0x3fff != 0 {
            return Err(Ipv4Error::Fragmented);
        }
        let mut source = [0; 4];
        source.copy_from_slice(&bytes[12..16]);
        let mut destination = [0; 4];
        destination.copy_from_slice(&bytes[16..20]);
        Ok(Self {
            source,
            destination,
            protocol: bytes[9],
            ttl: bytes[8],
            payload: &bytes[header_length..total_length],
        })
    }
}

fn internet_checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut chunks = bytes.chunks_exact(2);
    for chunk in &mut chunks {
        sum += u32::from(u16::from_be_bytes([chunk[0], chunk[1]]));
    }
    if let Some(byte) = chunks.remainder().first() {
        sum += u32::from(*byte) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn encode_ipv4_packet(
    out: &mut [u8; IPV4_MAX_PACKET_SIZE],
    source: [u8; 4],
    destination: [u8; 4],
    protocol: u8,
    payload: &[u8],
) -> Result<usize, Ipv4Error> {
    let total_length = IPV4_MIN_HEADER_SIZE + payload.len();
    if total_length > out.len() {
        return Err(Ipv4Error::PayloadTooLarge);
    }
    out.fill(0);
    out[0] = 0x45;
    out[2..4].copy_from_slice(&(total_length as u16).to_be_bytes());
    out[4..6].copy_from_slice(&1u16.to_be_bytes());
    out[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    out[8] = 64;
    out[9] = protocol;
    out[12..16].copy_from_slice(&source);
    out[16..20].copy_from_slice(&destination);
    let checksum = internet_checksum(&out[..IPV4_MIN_HEADER_SIZE]);
    out[10..12].copy_from_slice(&checksum.to_be_bytes());
    out[IPV4_MIN_HEADER_SIZE..total_length].copy_from_slice(payload);
    Ok(total_length)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArpPacket {
    pub operation: u16,
    pub sender_mac: [u8; 6],
    pub sender_ip: [u8; 4],
    pub target_mac: [u8; 6],
    pub target_ip: [u8; 4],
}

impl ArpPacket {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < ARP_PACKET_SIZE
            || u16::from_be_bytes([bytes[0], bytes[1]]) != ARP_HARDWARE_ETHERNET
            || u16::from_be_bytes([bytes[2], bytes[3]]) != ETHER_TYPE_IPV4
            || bytes[4] != 6
            || bytes[5] != 4
        {
            return None;
        }
        let operation = u16::from_be_bytes([bytes[6], bytes[7]]);
        if operation != ARP_OPERATION_REQUEST && operation != ARP_OPERATION_REPLY {
            return None;
        }
        let mut sender_mac = [0; 6];
        sender_mac.copy_from_slice(&bytes[8..14]);
        let mut sender_ip = [0; 4];
        sender_ip.copy_from_slice(&bytes[14..18]);
        let mut target_mac = [0; 6];
        target_mac.copy_from_slice(&bytes[18..24]);
        let mut target_ip = [0; 4];
        target_ip.copy_from_slice(&bytes[24..28]);
        Some(Self {
            operation,
            sender_mac,
            sender_ip,
            target_mac,
            target_ip,
        })
    }

    fn encode(self) -> [u8; ARP_PACKET_SIZE] {
        let mut bytes = [0; ARP_PACKET_SIZE];
        bytes[0..2].copy_from_slice(&ARP_HARDWARE_ETHERNET.to_be_bytes());
        bytes[2..4].copy_from_slice(&ETHER_TYPE_IPV4.to_be_bytes());
        bytes[4] = 6;
        bytes[5] = 4;
        bytes[6..8].copy_from_slice(&self.operation.to_be_bytes());
        bytes[8..14].copy_from_slice(&self.sender_mac);
        bytes[14..18].copy_from_slice(&self.sender_ip);
        bytes[18..24].copy_from_slice(&self.target_mac);
        bytes[24..28].copy_from_slice(&self.target_ip);
        bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EthernetProtocol {
    None,
    Ipv4,
    Arp,
    VantaraTest,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EthernetError {
    FrameTooShort,
    PayloadTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EthernetFrame<'a> {
    pub destination: [u8; 6],
    pub source: [u8; 6],
    pub ether_type: u16,
    pub payload: &'a [u8],
}

impl<'a> EthernetFrame<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, EthernetError> {
        if bytes.len() < ETHERNET_HEADER_SIZE {
            return Err(EthernetError::FrameTooShort);
        }
        let mut destination = [0u8; 6];
        destination.copy_from_slice(&bytes[..6]);
        let mut source = [0u8; 6];
        source.copy_from_slice(&bytes[6..12]);
        Ok(Self {
            destination,
            source,
            ether_type: u16::from_be_bytes([bytes[12], bytes[13]]),
            payload: &bytes[ETHERNET_HEADER_SIZE..],
        })
    }

    pub fn protocol(self) -> EthernetProtocol {
        match self.ether_type {
            ETHER_TYPE_IPV4 => EthernetProtocol::Ipv4,
            ETHER_TYPE_ARP => EthernetProtocol::Arp,
            ETHER_TYPE_VANTARA_TEST => EthernetProtocol::VantaraTest,
            _ => EthernetProtocol::Other,
        }
    }
}

fn encode_ethernet_frame(
    out: &mut [u8; ETHERNET_MIN_FRAME_SIZE],
    destination: [u8; 6],
    source: [u8; 6],
    ether_type: u16,
    payload: &[u8],
) -> Result<usize, EthernetError> {
    if ETHERNET_HEADER_SIZE + payload.len() > out.len() {
        return Err(EthernetError::PayloadTooLarge);
    }
    out.fill(0);
    out[..6].copy_from_slice(&destination);
    out[6..12].copy_from_slice(&source);
    out[12..14].copy_from_slice(&ether_type.to_be_bytes());
    out[ETHERNET_HEADER_SIZE..ETHERNET_HEADER_SIZE + payload.len()].copy_from_slice(payload);
    Ok(ETHERNET_MIN_FRAME_SIZE)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkKind {
    Ethernet,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverCandidate {
    IntelE1000,
    IntelE1000e,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkDevice {
    pub pci: PciDevice,
    pub kind: NetworkKind,
    pub driver: DriverCandidate,
    pub bar0: u64,
    pub mmio_ready: bool,
    pub control: u32,
    pub status: u32,
    pub link_up: bool,
    pub mac: [u8; 6],
    pub mac_valid: bool,
    pub rx_queue_ready: bool,
    pub tx_queue_ready: bool,
    pub queue_depth: u16,
    pub tx_test_ok: bool,
    pub tx_packets: u64,
    pub rx_test_ok: bool,
    pub rx_packets: u64,
    pub ethernet_frames: u64,
    pub last_ether_type: u16,
    pub last_protocol: EthernetProtocol,
    pub ipv4_address: [u8; 4],
    pub arp_requests: u64,
    pub arp_replies: u64,
    pub arp_cache: Option<([u8; 4], [u8; 6])>,
    pub ipv4_packets: u64,
    pub ipv4_checksum_valid: bool,
    pub last_ipv4_source: [u8; 4],
    pub last_ipv4_destination: [u8; 4],
    pub last_ipv4_protocol: u8,
    pub udp_packets: u64,
    pub udp_checksum_valid: bool,
    pub last_udp_source_port: u16,
    pub last_udp_destination_port: u16,
    pub udp_send_ok: bool,
    pub udp_socket_delivered: bool,
    mmio_base: u64,
    physical_memory_offset: u64,
    queues: Option<E1000Queues>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct E1000Queues {
    receive_ring: u64,
    transmit_ring: u64,
    receive_buffers: [u64; DMA_RING_DEPTH],
    transmit_buffers: [u64; DMA_RING_DEPTH],
    receive_next: u16,
    transmit_next: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ReceiveDescriptor {
    address: u64,
    length: u16,
    checksum: u16,
    status: u8,
    errors: u8,
    special: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TransmitDescriptor {
    address: u64,
    length: u16,
    checksum_offset: u8,
    command: u8,
    status: u8,
    checksum_start: u8,
    special: u16,
}

const _: () = {
    assert!(core::mem::size_of::<ReceiveDescriptor>() == 16);
    assert!(core::mem::size_of::<TransmitDescriptor>() == 16);
};

#[derive(Clone, Copy)]
struct E1000Mmio {
    base: u64,
}

impl E1000Mmio {
    fn read(self, offset: u64) -> u32 {
        debug_assert!(offset < E1000_MMIO_SIZE && offset % 4 == 0);
        let pointer = (self.base + offset) as *const u32;
        // SAFETY: `init` maps the complete e1000 register window and callers
        // only use aligned register offsets within that audited mapping.
        unsafe { core::ptr::read_volatile(pointer) }
    }

    fn write(self, offset: u64, value: u32) {
        debug_assert!(offset < E1000_MMIO_SIZE && offset % 4 == 0);
        let pointer = (self.base + offset) as *mut u32;
        // SAFETY: the same audited MMIO mapping and aligned-offset invariant
        // as `read` applies; initialization owns these controller registers.
        unsafe { core::ptr::write_volatile(pointer, value) };
    }
}

lazy_static! {
    static ref DEVICES: Mutex<Vec<NetworkDevice>> = Mutex::new(Vec::new());
    static ref UDP_SOCKETS: Mutex<UdpSocketTable> = Mutex::new(UdpSocketTable::new());
}

pub fn udp_bind(local_port: u16) -> Result<UdpSocketHandle, UdpSocketError> {
    UDP_SOCKETS.lock().bind(local_port)
}

pub fn udp_receive(handle: UdpSocketHandle) -> Result<ReceivedUdpDatagram, UdpSocketError> {
    UDP_SOCKETS.lock().receive(handle)
}

pub fn udp_close(handle: UdpSocketHandle) -> Result<(), UdpSocketError> {
    UDP_SOCKETS.lock().close(handle)
}

pub fn udp_send_to(
    handle: UdpSocketHandle,
    destination_ip: [u8; 4],
    destination_port: u16,
    payload: &[u8],
) -> Result<(), UdpSocketError> {
    let source_port = UDP_SOCKETS.lock().local_port(handle)?;
    let mut devices = DEVICES.lock();
    let device = devices
        .iter_mut()
        .find(|device| {
            device.driver != DriverCandidate::Unsupported
                && device.tx_queue_ready
                && device.ipv4_address != [0; 4]
        })
        .ok_or(UdpSocketError::NoRoute)?;
    submit_udp_datagram(
        device,
        destination_ip,
        source_port,
        destination_port,
        payload,
    )
}

/// Process packets already completed by the NIC without waiting inside the
/// runtime event loop. Work is bounded to one receive-ring pass per device.
pub fn poll_runtime() {
    let mut devices = DEVICES.lock();
    for device in devices.iter_mut() {
        for _ in 0..DMA_RING_DEPTH {
            if !poll_receive_with_limit(device, 1) {
                break;
            }
        }
    }
}

pub fn init(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    physical_memory_offset: VirtAddr,
) {
    let devices: Vec<NetworkDevice> = pci::devices_by_class(PCI_CLASS_NETWORK)
        .into_iter()
        .map(|device| {
            let driver = if device.vendor_id == INTEL_VENDOR_ID
                && device.device_id == INTEL_E1000_DEVICE_ID
            {
                DriverCandidate::IntelE1000
            } else if device.vendor_id == INTEL_VENDOR_ID
                && device.device_id == INTEL_E1000E_DEVICE_ID
            {
                DriverCandidate::IntelE1000e
            } else {
                DriverCandidate::Unsupported
            };
            let bar0 = match pci::read_bar_info(device, 0) {
                PciBar::Memory32 { address, .. } | PciBar::Memory64 { address, .. } => address,
                _ => 0,
            };
            NetworkDevice {
                kind: if device.subclass == PCI_SUBCLASS_ETHERNET {
                    NetworkKind::Ethernet
                } else {
                    NetworkKind::Other
                },
                driver,
                bar0,
                mmio_ready: false,
                control: 0,
                status: 0,
                link_up: false,
                mac: [0; 6],
                mac_valid: false,
                rx_queue_ready: false,
                tx_queue_ready: false,
                queue_depth: 0,
                tx_test_ok: false,
                tx_packets: 0,
                rx_test_ok: false,
                rx_packets: 0,
                ethernet_frames: 0,
                last_ether_type: 0,
                last_protocol: EthernetProtocol::None,
                ipv4_address: [0; 4],
                arp_requests: 0,
                arp_replies: 0,
                arp_cache: None,
                ipv4_packets: 0,
                ipv4_checksum_valid: false,
                last_ipv4_source: [0; 4],
                last_ipv4_destination: [0; 4],
                last_ipv4_protocol: 0,
                udp_packets: 0,
                udp_checksum_valid: false,
                last_udp_source_port: 0,
                last_udp_destination_port: 0,
                udp_send_ok: false,
                udp_socket_delivered: false,
                mmio_base: 0,
                physical_memory_offset: physical_memory_offset.as_u64(),
                queues: None,
                pci: device,
            }
        })
        .enumerate()
        .map(|(index, mut device)| {
            device.ipv4_address = [10, 0, 2, 15u8.saturating_add(index as u8)];
            if device.driver == DriverCandidate::Unsupported || device.bar0 == 0 {
                return device;
            }
            let Some(offset) = (index as u64).checked_mul(E1000_MMIO_STRIDE) else {
                return device;
            };
            let Some(mmio_base) = E1000_MMIO_WINDOW.checked_add(offset) else {
                return device;
            };
            if crate::memory::map_mmio_range(
                PhysAddr::new(device.bar0),
                VirtAddr::new(mmio_base),
                E1000_MMIO_SIZE,
                mapper,
                frame_allocator,
            )
            .is_err()
            {
                return device;
            }
            let registers = E1000Mmio { base: mmio_base };
            device.mmio_base = mmio_base;
            device.control = registers.read(REG_CTRL);
            device.status = registers.read(REG_STATUS);
            device.link_up = device.status & STATUS_LINK_UP != 0;
            let address_low = registers.read(REG_RECEIVE_ADDRESS_LOW);
            let address_high = registers.read(REG_RECEIVE_ADDRESS_HIGH);
            device.mac = [
                address_low as u8,
                (address_low >> 8) as u8,
                (address_low >> 16) as u8,
                (address_low >> 24) as u8,
                address_high as u8,
                (address_high >> 8) as u8,
            ];
            device.mac_valid = address_high & RECEIVE_ADDRESS_VALID != 0
                && device.mac.iter().any(|byte| *byte != 0);
            pci::enable_memory_and_bus_master(device.pci);
            if let Some(mut queues) =
                initialize_dma_queues(registers, frame_allocator, physical_memory_offset)
            {
                device.rx_queue_ready = true;
                device.tx_queue_ready = true;
                device.queue_depth = DMA_RING_DEPTH as u16;
                if submit_test_frame(registers, physical_memory_offset, &mut queues, device.mac) {
                    device.tx_test_ok = true;
                    device.tx_packets = 1;
                }
                device.queues = Some(queues);
            }
            device.mmio_ready = true;
            device
        })
        .collect();

    let mut devices = devices;
    let udp_receive_socket = udp_bind(UDP_TEST_DESTINATION_PORT).ok();
    let udp_send_socket = udp_bind(UDP_TEST_SOURCE_PORT).ok();
    for device in &mut devices {
        poll_receive(device);
    }
    if devices.len() >= 2 {
        let target_ip = devices[1].ipv4_address;
        submit_arp_request(&mut devices[0], target_ip);
        poll_receive(&mut devices[1]);
        poll_receive(&mut devices[0]);
        if let Some(handle) = udp_send_socket {
            let _ = submit_udp_from_socket(
                &mut devices[0],
                handle,
                target_ip,
                UDP_TEST_DESTINATION_PORT,
                UDP_TEST_PAYLOAD,
            );
            let _ = udp_close(handle);
        }
        poll_receive(&mut devices[1]);
        if let Some(handle) = udp_receive_socket
            && let Ok(datagram) = udp_receive(handle)
        {
            devices[1].udp_socket_delivered = datagram.source_ip == devices[0].ipv4_address
                && datagram.source_port == UDP_TEST_SOURCE_PORT
                && datagram.destination_port == UDP_TEST_DESTINATION_PORT
                && datagram.payload[..usize::from(datagram.payload_len)] == *UDP_TEST_PAYLOAD;
            let _ = udp_close(handle);
        }
    }

    crate::serial_println!("[NET] detected {} network device(s)", devices.len());
    let (state, detail) = if devices.is_empty() {
        (
            crate::drivers::status::DriverState::Missing,
            "no PCI network device",
        )
    } else if devices
        .iter()
        .any(|device| device.rx_queue_ready && device.tx_queue_ready && device.tx_test_ok)
    {
        (
            crate::drivers::status::DriverState::Degraded,
            "e1000 DMA rings ready; test Ethernet frame transmitted",
        )
    } else {
        (
            crate::drivers::status::DriverState::Degraded,
            "unsupported network hardware",
        )
    };
    crate::drivers::status::report("network", state, detail);

    for device in &devices {
        crate::serial_println!(
            "[NET] {:02x}:{:02x}.{} vendor={:04x} device={:04x} kind={:?} driver={:?} bar0={:#x} mmio_ready={} ctrl={:#010x} status={:#010x} link_up={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} mac_valid={} rxq={} txq={} depth={} tx_test={} tx_packets={} rx_test={} rx_packets={} ethernet_frames={} ether_type={:#06x} protocol={} ip={}.{}.{}.{} arp_requests={} arp_replies={} arp_resolved={} ipv4_packets={} ipv4_checksum={} ipv4_source={}.{}.{}.{} ipv4_destination={}.{}.{}.{} ipv4_protocol={} udp_packets={} udp_checksum={} udp_source_port={} udp_destination_port={} udp_send_ok={} udp_socket_delivered={}",
            device.pci.bus,
            device.pci.slot,
            device.pci.function,
            device.pci.vendor_id,
            device.pci.device_id,
            device.kind,
            device.driver,
            device.bar0,
            device.mmio_ready,
            device.control,
            device.status,
            device.link_up,
            device.mac[0],
            device.mac[1],
            device.mac[2],
            device.mac[3],
            device.mac[4],
            device.mac[5],
            device.mac_valid,
            device.rx_queue_ready,
            device.tx_queue_ready,
            device.queue_depth,
            device.tx_test_ok,
            device.tx_packets,
            device.rx_test_ok,
            device.rx_packets,
            device.ethernet_frames,
            device.last_ether_type,
            protocol_name(device.last_protocol),
            device.ipv4_address[0],
            device.ipv4_address[1],
            device.ipv4_address[2],
            device.ipv4_address[3],
            device.arp_requests,
            device.arp_replies,
            device.arp_cache.is_some(),
            device.ipv4_packets,
            device.ipv4_checksum_valid,
            device.last_ipv4_source[0],
            device.last_ipv4_source[1],
            device.last_ipv4_source[2],
            device.last_ipv4_source[3],
            device.last_ipv4_destination[0],
            device.last_ipv4_destination[1],
            device.last_ipv4_destination[2],
            device.last_ipv4_destination[3],
            device.last_ipv4_protocol,
            device.udp_packets,
            device.udp_checksum_valid,
            device.last_udp_source_port,
            device.last_udp_destination_port,
            device.udp_send_ok,
            device.udp_socket_delivered
        );
    }

    *DEVICES.lock() = devices;
}

fn initialize_dma_queues(
    registers: E1000Mmio,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    physical_memory_offset: VirtAddr,
) -> Option<E1000Queues> {
    let receive_ring = frame_allocator.allocate_frame()?.start_address().as_u64();
    let transmit_ring = frame_allocator.allocate_frame()?.start_address().as_u64();
    let mut receive_buffers = [0u64; DMA_RING_DEPTH];
    let mut transmit_buffers = [0u64; DMA_RING_DEPTH];
    for buffer in &mut receive_buffers {
        *buffer = frame_allocator.allocate_frame()?.start_address().as_u64();
    }
    for buffer in &mut transmit_buffers {
        *buffer = frame_allocator.allocate_frame()?.start_address().as_u64();
    }

    let receive_pointer = (physical_memory_offset + receive_ring).as_mut_ptr::<ReceiveDescriptor>();
    let transmit_pointer =
        (physical_memory_offset + transmit_ring).as_mut_ptr::<TransmitDescriptor>();
    // SAFETY: all frames were exclusively allocated for these DMA rings and
    // buffers. Descriptor indices stay within one page and each address names
    // its dedicated page-aligned packet buffer.
    unsafe {
        core::ptr::write_bytes(
            receive_pointer.cast::<u8>(),
            0,
            core::mem::size_of::<ReceiveDescriptor>() * DMA_RING_DEPTH,
        );
        core::ptr::write_bytes(
            transmit_pointer.cast::<u8>(),
            0,
            core::mem::size_of::<TransmitDescriptor>() * DMA_RING_DEPTH,
        );
        for index in 0..DMA_RING_DEPTH {
            core::ptr::write_volatile(
                receive_pointer.add(index),
                ReceiveDescriptor {
                    address: receive_buffers[index],
                    length: 0,
                    checksum: 0,
                    status: 0,
                    errors: 0,
                    special: 0,
                },
            );
            core::ptr::write_volatile(
                transmit_pointer.add(index),
                TransmitDescriptor {
                    address: transmit_buffers[index],
                    length: 0,
                    checksum_offset: 0,
                    command: 0,
                    status: 1,
                    checksum_start: 0,
                    special: 0,
                },
            );
        }
    }

    let ring_length = (core::mem::size_of::<ReceiveDescriptor>() * DMA_RING_DEPTH) as u32;
    registers.write(REG_INTERRUPT_MASK_CLEAR, u32::MAX);
    registers.write(REG_RECEIVE_DESC_LOW, receive_ring as u32);
    registers.write(REG_RECEIVE_DESC_HIGH, (receive_ring >> 32) as u32);
    registers.write(REG_RECEIVE_DESC_LENGTH, ring_length);
    registers.write(REG_RECEIVE_DESC_HEAD, 0);
    registers.write(REG_RECEIVE_DESC_TAIL, (DMA_RING_DEPTH - 1) as u32);
    registers.write(REG_TRANSMIT_DESC_LOW, transmit_ring as u32);
    registers.write(REG_TRANSMIT_DESC_HIGH, (transmit_ring >> 32) as u32);
    registers.write(REG_TRANSMIT_DESC_LENGTH, ring_length);
    registers.write(REG_TRANSMIT_DESC_HEAD, 0);
    registers.write(REG_TRANSMIT_DESC_TAIL, 0);
    registers.write(REG_TRANSMIT_IPG, 10 | (8 << 10) | (6 << 20));
    registers.write(
        REG_TRANSMIT_CONTROL,
        TRANSMIT_ENABLE | TRANSMIT_PAD_SHORT_PACKETS | (0x10 << 4) | (0x40 << 12),
    );
    registers.write(
        REG_RECEIVE_CONTROL,
        RECEIVE_ENABLE | RECEIVE_BROADCAST_ACCEPT | RECEIVE_STRIP_CRC,
    );

    Some(E1000Queues {
        receive_ring,
        transmit_ring,
        receive_buffers,
        transmit_buffers,
        receive_next: 0,
        transmit_next: 0,
    })
}

fn poll_receive(device: &mut NetworkDevice) {
    let _ = poll_receive_with_limit(device, TRANSMIT_POLL_LIMIT);
}

fn poll_receive_with_limit(device: &mut NetworkDevice, poll_limit: usize) -> bool {
    let Some(queues) = device.queues.as_mut() else {
        return false;
    };
    let index = queues.receive_next as usize;
    let physical_memory_offset = VirtAddr::new(device.physical_memory_offset);
    let descriptor_pointer =
        (physical_memory_offset + queues.receive_ring).as_mut_ptr::<ReceiveDescriptor>();
    let buffer_pointer = (physical_memory_offset + queues.receive_buffers[index]).as_ptr::<u8>();

    // SAFETY: the receive descriptor and packet frame belong to this queue;
    // status is checked before bounded reads, then ownership is returned to the
    // NIC by clearing status and advancing the receive tail.
    let received_arp;
    let received_ipv4;
    unsafe {
        let mut completed = None;
        for _ in 0..poll_limit {
            let descriptor = core::ptr::read_volatile(descriptor_pointer.add(index));
            if descriptor.status & DESCRIPTOR_DONE != 0 {
                completed = Some(descriptor);
                break;
            }
            core::hint::spin_loop();
        }
        let Some(descriptor) = completed else {
            return false;
        };
        if descriptor.errors != 0
            || usize::from(descriptor.length) < 27
            || usize::from(descriptor.length) > 2048
        {
            return false;
        }
        let packet = core::slice::from_raw_parts(buffer_pointer, usize::from(descriptor.length));
        let parsed = EthernetFrame::parse(packet).ok();
        let protocol = parsed
            .map(EthernetFrame::protocol)
            .unwrap_or(EthernetProtocol::None);
        let ether_type = parsed.map_or(0, |frame| frame.ether_type);
        let valid = parsed.is_some_and(|frame| {
            frame.destination == [0xff; 6]
                && frame.protocol() == EthernetProtocol::VantaraTest
                && frame.payload.starts_with(TEST_PAYLOAD)
        });
        received_arp = parsed
            .filter(|frame| frame.protocol() == EthernetProtocol::Arp)
            .and_then(|frame| ArpPacket::parse(frame.payload));
        received_ipv4 = parsed
            .filter(|frame| frame.protocol() == EthernetProtocol::Ipv4)
            .and_then(|frame| Ipv4Packet::parse(frame.payload).ok())
            .map(|packet| {
                let udp = if packet.protocol == IP_PROTOCOL_UDP {
                    UdpDatagram::parse(packet.source, packet.destination, packet.payload)
                        .ok()
                        .map(|datagram| {
                            let mut payload = [0; UDP_MAX_PAYLOAD_SIZE];
                            payload[..datagram.payload.len()].copy_from_slice(datagram.payload);
                            (
                                datagram.source_port,
                                datagram.destination_port,
                                datagram.payload.starts_with(UDP_TEST_PAYLOAD),
                                payload,
                                datagram.payload.len() as u8,
                            )
                        })
                } else {
                    None
                };
                (packet.source, packet.destination, packet.protocol, udp)
            });
        core::ptr::write_volatile(
            descriptor_pointer.add(index),
            ReceiveDescriptor {
                address: queues.receive_buffers[index],
                length: 0,
                checksum: 0,
                status: 0,
                errors: 0,
                special: 0,
            },
        );
        fence(Ordering::SeqCst);
        E1000Mmio {
            base: device.mmio_base,
        }
        .write(REG_RECEIVE_DESC_TAIL, index as u32);
        queues.receive_next = ((index + 1) % DMA_RING_DEPTH) as u16;
        device.rx_packets = device.rx_packets.saturating_add(1);
        device.ethernet_frames = device
            .ethernet_frames
            .saturating_add(parsed.is_some() as u64);
        device.last_ether_type = ether_type;
        device.last_protocol = protocol;
        device.rx_test_ok |= valid;
    }
    if let Some(packet) = received_arp {
        device.arp_cache = Some((packet.sender_ip, packet.sender_mac));
        if packet.operation == ARP_OPERATION_REQUEST && packet.target_ip == device.ipv4_address {
            device.arp_requests = device.arp_requests.saturating_add(1);
            submit_arp_reply(device, packet.sender_mac, packet.sender_ip);
        } else if packet.operation == ARP_OPERATION_REPLY && packet.target_ip == device.ipv4_address
        {
            device.arp_replies = device.arp_replies.saturating_add(1);
        }
    }
    if let Some((source, destination, protocol, udp)) = received_ipv4 {
        if destination == device.ipv4_address {
            device.ipv4_packets = device.ipv4_packets.saturating_add(1);
            device.ipv4_checksum_valid = true;
            device.last_ipv4_source = source;
            device.last_ipv4_destination = destination;
            device.last_ipv4_protocol = protocol;
            if let Some((source_port, destination_port, test_payload_valid, payload, payload_len)) =
                udp
            {
                device.udp_packets = device.udp_packets.saturating_add(1);
                device.udp_checksum_valid = true;
                device.last_udp_source_port = source_port;
                device.last_udp_destination_port = destination_port;
                device.rx_test_ok |= source_port == UDP_TEST_SOURCE_PORT
                    && destination_port == UDP_TEST_DESTINATION_PORT
                    && test_payload_valid;
                UDP_SOCKETS.lock().deliver(ReceivedUdpDatagram {
                    source_ip: source,
                    source_port,
                    destination_port,
                    payload,
                    payload_len,
                });
            }
        }
    }
    true
}

fn submit_udp_from_socket(
    device: &mut NetworkDevice,
    handle: UdpSocketHandle,
    destination_ip: [u8; 4],
    destination_port: u16,
    payload: &[u8],
) -> Result<(), UdpSocketError> {
    let source_port = UDP_SOCKETS.lock().local_port(handle)?;
    submit_udp_datagram(
        device,
        destination_ip,
        source_port,
        destination_port,
        payload,
    )
}

fn submit_udp_datagram(
    device: &mut NetworkDevice,
    destination_ip: [u8; 4],
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
) -> Result<(), UdpSocketError> {
    if source_port == 0 || destination_port == 0 {
        return Err(UdpSocketError::InvalidPort);
    }
    let Some((cached_ip, destination_mac)) = device.arp_cache else {
        return Err(UdpSocketError::ArpUnresolved);
    };
    if cached_ip != destination_ip {
        return Err(UdpSocketError::ArpUnresolved);
    }
    let mut datagram = [0; UDP_MAX_DATAGRAM_SIZE];
    let datagram_len = encode_udp_datagram(
        &mut datagram,
        device.ipv4_address,
        destination_ip,
        source_port,
        destination_port,
        payload,
    )
    .map_err(|error| match error {
        UdpError::PayloadTooLarge => UdpSocketError::PayloadTooLarge,
        _ => UdpSocketError::TransmitFailed,
    })?;
    let mut packet = [0; IPV4_MAX_PACKET_SIZE];
    let packet_len = encode_ipv4_packet(
        &mut packet,
        device.ipv4_address,
        destination_ip,
        IP_PROTOCOL_UDP,
        &datagram[..datagram_len],
    )
    .map_err(|error| match error {
        Ipv4Error::PayloadTooLarge => UdpSocketError::PayloadTooLarge,
        _ => UdpSocketError::TransmitFailed,
    })?;
    let mut frame = [0; ETHERNET_MIN_FRAME_SIZE];
    if encode_ethernet_frame(
        &mut frame,
        destination_mac,
        device.mac,
        ETHER_TYPE_IPV4,
        &packet[..packet_len],
    )
    .is_err()
    {
        return Err(UdpSocketError::TransmitFailed);
    }
    let Some(queues) = device.queues.as_mut() else {
        return Err(UdpSocketError::NoRoute);
    };
    let sent = submit_frame(
        E1000Mmio {
            base: device.mmio_base,
        },
        VirtAddr::new(device.physical_memory_offset),
        queues,
        &frame,
    );
    if sent {
        device.tx_packets = device.tx_packets.saturating_add(1);
        device.udp_send_ok = true;
        Ok(())
    } else {
        Err(UdpSocketError::TransmitFailed)
    }
}

fn submit_arp_request(device: &mut NetworkDevice, target_ip: [u8; 4]) {
    let packet = ArpPacket {
        operation: ARP_OPERATION_REQUEST,
        sender_mac: device.mac,
        sender_ip: device.ipv4_address,
        target_mac: [0; 6],
        target_ip,
    };
    submit_arp(device, [0xff; 6], packet);
}

fn submit_arp_reply(device: &mut NetworkDevice, target_mac: [u8; 6], target_ip: [u8; 4]) {
    let packet = ArpPacket {
        operation: ARP_OPERATION_REPLY,
        sender_mac: device.mac,
        sender_ip: device.ipv4_address,
        target_mac,
        target_ip,
    };
    if submit_arp(device, target_mac, packet) {
        device.arp_replies = device.arp_replies.saturating_add(1);
    }
}

fn submit_arp(device: &mut NetworkDevice, destination: [u8; 6], packet: ArpPacket) -> bool {
    let Some(queues) = device.queues.as_mut() else {
        return false;
    };
    let mut frame = [0; ETHERNET_MIN_FRAME_SIZE];
    let payload = packet.encode();
    if encode_ethernet_frame(
        &mut frame,
        destination,
        device.mac,
        ETHER_TYPE_ARP,
        &payload,
    )
    .is_err()
    {
        return false;
    }
    let sent = submit_frame(
        E1000Mmio {
            base: device.mmio_base,
        },
        VirtAddr::new(device.physical_memory_offset),
        queues,
        &frame,
    );
    if sent {
        device.tx_packets = device.tx_packets.saturating_add(1);
    }
    sent
}

fn submit_test_frame(
    registers: E1000Mmio,
    physical_memory_offset: VirtAddr,
    queues: &mut E1000Queues,
    source_mac: [u8; 6],
) -> bool {
    let mut frame = [0u8; ETHERNET_MIN_FRAME_SIZE];
    let Ok(frame_len) = encode_ethernet_frame(
        &mut frame,
        [0xff; 6],
        source_mac,
        ETHER_TYPE_VANTARA_TEST,
        TEST_PAYLOAD,
    ) else {
        return false;
    };

    submit_frame(
        registers,
        physical_memory_offset,
        queues,
        &frame[..frame_len],
    )
}

fn submit_frame(
    registers: E1000Mmio,
    physical_memory_offset: VirtAddr,
    queues: &mut E1000Queues,
    frame: &[u8],
) -> bool {
    let index = queues.transmit_next as usize;
    let descriptor_pointer =
        (physical_memory_offset + queues.transmit_ring).as_mut_ptr::<TransmitDescriptor>();
    let buffer_pointer =
        (physical_memory_offset + queues.transmit_buffers[index]).as_mut_ptr::<u8>();
    // SAFETY: the selected TX buffer and descriptor belong exclusively to the
    // initialized ring. The 60-byte frame fits its page and the descriptor
    // index is bounded by `DMA_RING_DEPTH`.
    unsafe {
        core::ptr::copy_nonoverlapping(frame.as_ptr(), buffer_pointer, frame.len());
        core::ptr::write_volatile(
            descriptor_pointer.add(index),
            TransmitDescriptor {
                address: queues.transmit_buffers[index],
                length: frame.len() as u16,
                checksum_offset: 0,
                command: TRANSMIT_COMMAND_EOP_IFCS_RS,
                status: 0,
                checksum_start: 0,
                special: 0,
            },
        );
        fence(Ordering::SeqCst);
        let next = (index + 1) % DMA_RING_DEPTH;
        registers.write(REG_TRANSMIT_DESC_TAIL, next as u32);
        for _ in 0..TRANSMIT_POLL_LIMIT {
            let descriptor = core::ptr::read_volatile(descriptor_pointer.add(index));
            if descriptor.status & DESCRIPTOR_DONE != 0 {
                queues.transmit_next = next as u16;
                return true;
            }
            core::hint::spin_loop();
        }
    }
    false
}

pub fn write_devices_to_buffer(out: &mut [u8]) -> usize {
    let devices = DEVICES.lock();
    let mut writer = BufferWriter::new(out);

    writer.write_str("Network devices: ");
    writer.write_dec(devices.len() as u64);
    writer.write_byte(b'\n');
    writer.write_str(
        "BDF       VENDOR:DEVICE TYPE      DRIVER       BAR0             MMIO LINK MAC               RXQ TXQ DEPTH TXOK TXPACKETS RXOK RXPACKETS ETHFRAMES ETHERTYPE PROTOCOL IP ARPREQ ARPREPLY RESOLVED IPV4 CHECKSUM SOURCE DESTINATION IPPROTO UDP UDPCHECK SPORT DPORT UDPSEND SOCKET\n",
    );

    for device in devices.iter() {
        writer.write_hex_u8(device.pci.bus);
        writer.write_byte(b':');
        writer.write_hex_u8(device.pci.slot);
        writer.write_byte(b'.');
        writer.write_dec(device.pci.function as u64);
        writer.write_str("  ");
        writer.write_hex_u16(device.pci.vendor_id);
        writer.write_byte(b':');
        writer.write_hex_u16(device.pci.device_id);
        writer.write_str(" ");
        writer.write_padded(kind_name(device.kind), 9);
        writer.write_byte(b' ');
        writer.write_padded(driver_name(device.driver), 12);
        writer.write_str(" 0x");
        writer.write_hex_u64(device.bar0);
        writer.write_byte(b' ');
        writer.write_dec(device.mmio_ready as u64);
        writer.write_byte(b' ');
        writer.write_dec(device.link_up as u64);
        writer.write_byte(b' ');
        writer.write_mac(device.mac);
        writer.write_byte(b' ');
        writer.write_dec(device.rx_queue_ready as u64);
        writer.write_byte(b' ');
        writer.write_dec(device.tx_queue_ready as u64);
        writer.write_byte(b' ');
        writer.write_dec(device.queue_depth as u64);
        writer.write_byte(b' ');
        writer.write_dec(device.tx_test_ok as u64);
        writer.write_byte(b' ');
        writer.write_dec(device.tx_packets);
        writer.write_byte(b' ');
        writer.write_dec(device.rx_test_ok as u64);
        writer.write_byte(b' ');
        writer.write_dec(device.rx_packets);
        writer.write_byte(b' ');
        writer.write_dec(device.ethernet_frames);
        writer.write_str(" 0x");
        writer.write_hex_u16(device.last_ether_type);
        writer.write_byte(b' ');
        writer.write_str(protocol_name(device.last_protocol));
        writer.write_byte(b' ');
        writer.write_ipv4(device.ipv4_address);
        writer.write_byte(b' ');
        writer.write_dec(device.arp_requests);
        writer.write_byte(b' ');
        writer.write_dec(device.arp_replies);
        writer.write_byte(b' ');
        writer.write_dec(device.arp_cache.is_some() as u64);
        writer.write_byte(b' ');
        writer.write_dec(device.ipv4_packets);
        writer.write_byte(b' ');
        writer.write_dec(device.ipv4_checksum_valid as u64);
        writer.write_byte(b' ');
        writer.write_ipv4(device.last_ipv4_source);
        writer.write_byte(b' ');
        writer.write_ipv4(device.last_ipv4_destination);
        writer.write_byte(b' ');
        writer.write_dec(device.last_ipv4_protocol as u64);
        writer.write_byte(b' ');
        writer.write_dec(device.udp_packets);
        writer.write_byte(b' ');
        writer.write_dec(device.udp_checksum_valid as u64);
        writer.write_byte(b' ');
        writer.write_dec(device.last_udp_source_port as u64);
        writer.write_byte(b' ');
        writer.write_dec(device.last_udp_destination_port as u64);
        writer.write_byte(b' ');
        writer.write_dec(device.udp_send_ok as u64);
        writer.write_byte(b' ');
        writer.write_dec(device.udp_socket_delivered as u64);
        writer.write_byte(b'\n');
    }

    writer.len()
}

fn kind_name(kind: NetworkKind) -> &'static str {
    match kind {
        NetworkKind::Ethernet => "ethernet",
        NetworkKind::Other => "other",
    }
}

fn driver_name(driver: DriverCandidate) -> &'static str {
    match driver {
        DriverCandidate::IntelE1000 => "e1000",
        DriverCandidate::IntelE1000e => "e1000e",
        DriverCandidate::Unsupported => "unsupported",
    }
}

fn protocol_name(protocol: EthernetProtocol) -> &'static str {
    match protocol {
        EthernetProtocol::None => "none",
        EthernetProtocol::Ipv4 => "ipv4",
        EthernetProtocol::Arp => "arp",
        EthernetProtocol::VantaraTest => "vantara-test",
        EthernetProtocol::Other => "other",
    }
}

struct BufferWriter<'a> {
    out: &'a mut [u8],
    len: usize,
}

impl<'a> BufferWriter<'a> {
    fn new(out: &'a mut [u8]) -> Self {
        Self { out, len: 0 }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn write_byte(&mut self, byte: u8) {
        if self.len < self.out.len() {
            self.out[self.len] = byte;
            self.len += 1;
        }
    }

    fn write_str(&mut self, text: &str) {
        for byte in text.bytes() {
            self.write_byte(byte);
        }
    }

    fn write_padded(&mut self, text: &str, width: usize) {
        self.write_str(text);
        for _ in text.len()..width {
            self.write_byte(b' ');
        }
    }

    fn write_dec(&mut self, mut value: u64) {
        let mut digits = [0u8; 20];
        if value == 0 {
            self.write_byte(b'0');
            return;
        }

        let mut len = 0usize;
        while value > 0 {
            digits[len] = b'0' + (value % 10) as u8;
            value /= 10;
            len += 1;
        }
        while len > 0 {
            len -= 1;
            self.write_byte(digits[len]);
        }
    }

    fn write_hex_u8(&mut self, value: u8) {
        self.write_hex_nibble(value >> 4);
        self.write_hex_nibble(value);
    }

    fn write_hex_u16(&mut self, value: u16) {
        self.write_hex_u8((value >> 8) as u8);
        self.write_hex_u8(value as u8);
    }

    fn write_hex_u32(&mut self, value: u32) {
        self.write_hex_u16((value >> 16) as u16);
        self.write_hex_u16(value as u16);
    }

    fn write_hex_u64(&mut self, value: u64) {
        self.write_hex_u32((value >> 32) as u32);
        self.write_hex_u32(value as u32);
    }

    fn write_mac(&mut self, mac: [u8; 6]) {
        for (index, byte) in mac.iter().enumerate() {
            if index != 0 {
                self.write_byte(b':');
            }
            self.write_hex_u8(*byte);
        }
    }

    fn write_ipv4(&mut self, address: [u8; 4]) {
        for (index, octet) in address.iter().enumerate() {
            if index != 0 {
                self.write_byte(b'.');
            }
            self.write_dec(*octet as u64);
        }
    }

    fn write_hex_nibble(&mut self, value: u8) {
        self.write_byte(match value & 0x0f {
            digit @ 0..=9 => b'0' + digit,
            digit => b'a' + digit - 10,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ARP_OPERATION_REPLY, ARP_OPERATION_REQUEST, ArpPacket, BufferWriter, DriverCandidate,
        ETHER_TYPE_ARP, ETHER_TYPE_IPV4, ETHER_TYPE_VANTARA_TEST, ETHERNET_MIN_FRAME_SIZE,
        EthernetError, EthernetFrame, EthernetProtocol, IP_PROTOCOL_UDP, IPV4_MAX_PACKET_SIZE,
        Ipv4Error, Ipv4Packet, NetworkKind, ReceivedUdpDatagram, UDP_MAX_DATAGRAM_SIZE,
        UDP_SOCKET_QUEUE_DEPTH, UdpDatagram, UdpError, UdpSocketError, UdpSocketTable, driver_name,
        encode_ethernet_frame, encode_ipv4_packet, encode_udp_datagram, kind_name, protocol_name,
    };

    #[test_case]
    fn exposes_stable_network_labels() {
        assert_eq!(kind_name(NetworkKind::Ethernet), "ethernet");
        assert_eq!(driver_name(DriverCandidate::IntelE1000), "e1000");
    }

    #[test_case]
    fn formats_network_bar() {
        let mut buffer = [0u8; 16];
        let mut writer = BufferWriter::new(&mut buffer);
        writer.write_hex_u32(0xfebc_0000);
        let len = writer.len();
        drop(writer);
        assert_eq!(&buffer[..len], b"febc0000");
    }

    #[test_case]
    fn encodes_and_parses_ethernet_frame() {
        let mut bytes = [0u8; ETHERNET_MIN_FRAME_SIZE];
        let destination = [0xff; 6];
        let source = [0x02, 0, 0, 0, 0, 1];
        let len = encode_ethernet_frame(
            &mut bytes,
            destination,
            source,
            ETHER_TYPE_VANTARA_TEST,
            b"hello",
        )
        .unwrap();
        let frame = EthernetFrame::parse(&bytes[..len]).unwrap();
        assert_eq!(frame.destination, destination);
        assert_eq!(frame.source, source);
        assert_eq!(frame.protocol(), EthernetProtocol::VantaraTest);
        assert_eq!(&frame.payload[..5], b"hello");
    }

    #[test_case]
    fn dispatches_standard_ether_types() {
        let mut bytes = [0u8; ETHERNET_MIN_FRAME_SIZE];
        bytes[12..14].copy_from_slice(&ETHER_TYPE_ARP.to_be_bytes());
        assert_eq!(
            EthernetFrame::parse(&bytes).unwrap().protocol(),
            EthernetProtocol::Arp
        );
        bytes[12..14].copy_from_slice(&ETHER_TYPE_IPV4.to_be_bytes());
        assert_eq!(
            EthernetFrame::parse(&bytes).unwrap().protocol(),
            EthernetProtocol::Ipv4
        );
        assert_eq!(protocol_name(EthernetProtocol::Ipv4), "ipv4");
    }

    #[test_case]
    fn rejects_short_or_oversized_ethernet_frames() {
        assert_eq!(
            EthernetFrame::parse(&[0u8; 13]),
            Err(EthernetError::FrameTooShort)
        );
        let mut bytes = [0u8; ETHERNET_MIN_FRAME_SIZE];
        assert_eq!(
            encode_ethernet_frame(&mut bytes, [0; 6], [0; 6], 0, &[0; 47]),
            Err(EthernetError::PayloadTooLarge)
        );
    }

    #[test_case]
    fn encodes_and_parses_arp_request_and_reply() {
        for operation in [ARP_OPERATION_REQUEST, ARP_OPERATION_REPLY] {
            let packet = ArpPacket {
                operation,
                sender_mac: [0x52, 0x54, 0, 0x12, 0x34, 0x56],
                sender_ip: [10, 0, 2, 15],
                target_mac: [0; 6],
                target_ip: [10, 0, 2, 16],
            };
            assert_eq!(ArpPacket::parse(&packet.encode()), Some(packet));
        }
    }

    #[test_case]
    fn rejects_malformed_arp_packets() {
        assert_eq!(ArpPacket::parse(&[0; 27]), None);
        let mut packet = ArpPacket {
            operation: ARP_OPERATION_REQUEST,
            sender_mac: [1; 6],
            sender_ip: [10, 0, 2, 15],
            target_mac: [0; 6],
            target_ip: [10, 0, 2, 16],
        }
        .encode();
        packet[4] = 5;
        assert_eq!(ArpPacket::parse(&packet), None);
    }

    #[test_case]
    fn encodes_parses_and_checksums_ipv4_packet() {
        let mut bytes = [0; IPV4_MAX_PACKET_SIZE];
        let length = encode_ipv4_packet(
            &mut bytes,
            [10, 0, 2, 15],
            [10, 0, 2, 16],
            IP_PROTOCOL_UDP,
            b"hello",
        )
        .unwrap();
        let packet = Ipv4Packet::parse(&bytes[..length]).unwrap();
        assert_eq!(packet.source, [10, 0, 2, 15]);
        assert_eq!(packet.destination, [10, 0, 2, 16]);
        assert_eq!(packet.protocol, IP_PROTOCOL_UDP);
        assert_eq!(packet.ttl, 64);
        assert_eq!(packet.payload, b"hello");
    }

    #[test_case]
    fn rejects_bad_ipv4_checksum_and_fragments() {
        let mut bytes = [0; IPV4_MAX_PACKET_SIZE];
        let length = encode_ipv4_packet(&mut bytes, [1; 4], [2; 4], 17, b"udp").unwrap();
        bytes[10] ^= 1;
        assert_eq!(
            Ipv4Packet::parse(&bytes[..length]),
            Err(Ipv4Error::InvalidChecksum)
        );

        let length = encode_ipv4_packet(&mut bytes, [1; 4], [2; 4], 17, b"udp").unwrap();
        bytes[6..8].copy_from_slice(&0x2000u16.to_be_bytes());
        bytes[10..12].fill(0);
        let checksum = super::internet_checksum(&bytes[..20]);
        bytes[10..12].copy_from_slice(&checksum.to_be_bytes());
        assert_eq!(
            Ipv4Packet::parse(&bytes[..length]),
            Err(Ipv4Error::Fragmented)
        );
    }

    #[test_case]
    fn rejects_oversized_ipv4_payload() {
        let mut bytes = [0; IPV4_MAX_PACKET_SIZE];
        assert_eq!(
            encode_ipv4_packet(&mut bytes, [1; 4], [2; 4], 17, &[0; 27]),
            Err(Ipv4Error::PayloadTooLarge)
        );
    }

    #[test_case]
    fn encodes_parses_and_checksums_udp_datagram() {
        let source = [10, 0, 2, 15];
        let destination = [10, 0, 2, 16];
        let mut bytes = [0; UDP_MAX_DATAGRAM_SIZE];
        let length =
            encode_udp_datagram(&mut bytes, source, destination, 40000, 7777, b"hello").unwrap();
        let datagram = UdpDatagram::parse(source, destination, &bytes[..length]).unwrap();
        assert_eq!(datagram.source_port, 40000);
        assert_eq!(datagram.destination_port, 7777);
        assert_eq!(datagram.payload, b"hello");
    }

    #[test_case]
    fn rejects_bad_udp_checksum_and_length() {
        let source = [10, 0, 2, 15];
        let destination = [10, 0, 2, 16];
        let mut bytes = [0; UDP_MAX_DATAGRAM_SIZE];
        let length = encode_udp_datagram(&mut bytes, source, destination, 1, 2, b"x").unwrap();
        bytes[6] ^= 1;
        assert_eq!(
            UdpDatagram::parse(source, destination, &bytes[..length]),
            Err(UdpError::InvalidChecksum)
        );
        bytes[4..6].copy_from_slice(&7u16.to_be_bytes());
        assert_eq!(
            UdpDatagram::parse(source, destination, &bytes[..length]),
            Err(UdpError::InvalidLength)
        );
    }

    #[test_case]
    fn rejects_oversized_udp_payload() {
        let mut bytes = [0; UDP_MAX_DATAGRAM_SIZE];
        assert_eq!(
            encode_udp_datagram(&mut bytes, [1; 4], [2; 4], 1, 2, &[0; 19]),
            Err(UdpError::PayloadTooLarge)
        );
    }

    #[test_case]
    fn binds_queues_receives_and_closes_udp_socket() {
        let mut sockets = UdpSocketTable::new();
        let handle = sockets.bind(7777).unwrap();
        assert_eq!(sockets.bind(7777), Err(UdpSocketError::AddressInUse));
        for value in 0..UDP_SOCKET_QUEUE_DEPTH {
            assert!(sockets.deliver(ReceivedUdpDatagram {
                source_ip: [10, 0, 2, 15],
                source_port: 40000,
                destination_port: 7777,
                payload: [value as u8; super::UDP_MAX_PAYLOAD_SIZE],
                payload_len: 1,
            }));
        }
        assert!(!sockets.deliver(ReceivedUdpDatagram {
            source_ip: [10, 0, 2, 15],
            source_port: 40000,
            destination_port: 7777,
            payload: [9; super::UDP_MAX_PAYLOAD_SIZE],
            payload_len: 1,
        }));
        assert_eq!(sockets.receive(handle).unwrap().payload[0], 0);
        assert_eq!(sockets.local_port(handle), Ok(7777));
        sockets.close(handle).unwrap();
        assert_eq!(sockets.receive(handle), Err(UdpSocketError::InvalidHandle));
        assert_eq!(
            sockets.local_port(handle),
            Err(UdpSocketError::InvalidHandle)
        );
    }
}
