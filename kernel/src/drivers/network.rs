use crate::sync::PreemptMutex as Mutex;
use alloc::vec::Vec;
use lazy_static::lazy_static;

use crate::drivers::pci::{self, PciDevice};

const PCI_CLASS_NETWORK: u8 = 0x02;
const PCI_SUBCLASS_ETHERNET: u8 = 0x00;
const INTEL_VENDOR_ID: u16 = 0x8086;
const INTEL_E1000_DEVICE_ID: u16 = 0x100e;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkKind {
    Ethernet,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverCandidate {
    IntelE1000,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkDevice {
    pub pci: PciDevice,
    pub kind: NetworkKind,
    pub driver: DriverCandidate,
    pub bar0: u32,
}

lazy_static! {
    static ref DEVICES: Mutex<Vec<NetworkDevice>> = Mutex::new(Vec::new());
}

pub fn init() {
    let devices: Vec<NetworkDevice> = pci::devices_by_class(PCI_CLASS_NETWORK)
        .into_iter()
        .map(|device| NetworkDevice {
            kind: if device.subclass == PCI_SUBCLASS_ETHERNET {
                NetworkKind::Ethernet
            } else {
                NetworkKind::Other
            },
            driver: if device.vendor_id == INTEL_VENDOR_ID
                && device.device_id == INTEL_E1000_DEVICE_ID
            {
                DriverCandidate::IntelE1000
            } else {
                DriverCandidate::Unsupported
            },
            bar0: pci::read_bar(device, 0),
            pci: device,
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
        .any(|device| device.driver == DriverCandidate::IntelE1000)
    {
        (
            crate::drivers::status::DriverState::Degraded,
            "e1000 detected; packet driver pending",
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
            "[NET] {:02x}:{:02x}.{} vendor={:04x} device={:04x} kind={:?} driver={:?} bar0={:#x}",
            device.pci.bus,
            device.pci.slot,
            device.pci.function,
            device.pci.vendor_id,
            device.pci.device_id,
            device.kind,
            device.driver,
            device.bar0
        );
    }

    *DEVICES.lock() = devices;
}

pub fn write_devices_to_buffer(out: &mut [u8]) -> usize {
    let devices = DEVICES.lock();
    let mut writer = BufferWriter::new(out);

    writer.write_str("Network devices: ");
    writer.write_dec(devices.len() as u64);
    writer.write_byte(b'\n');
    writer.write_str("BDF       VENDOR:DEVICE TYPE      DRIVER       BAR0\n");

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
        writer.write_hex_u32(device.bar0);
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
