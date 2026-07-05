use crate::sync::PreemptMutex as Mutex;
use alloc::vec::Vec;
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
    queues: Option<E1000Queues>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct E1000Queues {
    receive_ring: u64,
    transmit_ring: u64,
    receive_buffers: [u64; DMA_RING_DEPTH],
    transmit_buffers: [u64; DMA_RING_DEPTH],
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
                queues: None,
                pci: device,
            }
        })
        .enumerate()
        .map(|(index, mut device)| {
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
            if let Some(queues) =
                initialize_dma_queues(registers, frame_allocator, physical_memory_offset)
            {
                device.rx_queue_ready = true;
                device.tx_queue_ready = true;
                device.queue_depth = DMA_RING_DEPTH as u16;
                device.queues = Some(queues);
            }
            device.mmio_ready = true;
            device
        })
        .collect();

    crate::serial_println!("[NET] detected {} network device(s)", devices.len());
    let (state, detail) = if devices.is_empty() {
        (
            crate::drivers::status::DriverState::Missing,
            "no PCI network device",
        )
    } else if devices
        .iter()
        .any(|device| device.rx_queue_ready && device.tx_queue_ready)
    {
        (
            crate::drivers::status::DriverState::Degraded,
            "e1000 RX/TX DMA rings ready; packet API pending",
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
            "[NET] {:02x}:{:02x}.{} vendor={:04x} device={:04x} kind={:?} driver={:?} bar0={:#x} mmio_ready={} ctrl={:#010x} status={:#010x} link_up={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} mac_valid={} rxq={} txq={} depth={}",
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
            device.queue_depth
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
    })
}

pub fn write_devices_to_buffer(out: &mut [u8]) -> usize {
    let devices = DEVICES.lock();
    let mut writer = BufferWriter::new(out);

    writer.write_str("Network devices: ");
    writer.write_dec(devices.len() as u64);
    writer.write_byte(b'\n');
    writer.write_str(
        "BDF       VENDOR:DEVICE TYPE      DRIVER       BAR0             MMIO LINK MAC               RXQ TXQ DEPTH\n",
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

    fn write_hex_nibble(&mut self, value: u8) {
        self.write_byte(match value & 0x0f {
            digit @ 0..=9 => b'0' + digit,
            digit => b'a' + digit - 10,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{BufferWriter, DriverCandidate, NetworkKind, driver_name, kind_name};

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
}
