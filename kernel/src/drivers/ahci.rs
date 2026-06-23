use alloc::vec::Vec;
use lazy_static::lazy_static;

use crate::drivers::pci::{self, PciBar, PciDevice};
use crate::sync::PreemptMutex as Mutex;

const PCI_CLASS_MASS_STORAGE: u8 = 0x01;
const PCI_SUBCLASS_SATA: u8 = 0x06;
const PCI_PROG_IF_AHCI: u8 = 0x01;
const AHCI_ABAR_INDEX: u8 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AhciController {
    pub pci: PciDevice,
    pub abar: u64,
    pub prefetchable: bool,
    pub mmio_ready: bool,
}

lazy_static! {
    static ref CONTROLLERS: Mutex<Vec<AhciController>> = Mutex::new(Vec::new());
}

pub fn init() {
    let controllers: Vec<AhciController> = pci::devices_by_class(PCI_CLASS_MASS_STORAGE)
        .into_iter()
        .filter(|device| device.subclass == PCI_SUBCLASS_SATA && device.prog_if == PCI_PROG_IF_AHCI)
        .filter_map(controller_from_pci)
        .collect();

    if controllers.is_empty() {
        crate::drivers::status::report(
            "ahci",
            crate::drivers::status::DriverState::Missing,
            "no PCI AHCI controller",
        );
        crate::serial_println!("[AHCI] no controller detected");
    } else {
        crate::drivers::status::report(
            "ahci",
            crate::drivers::status::DriverState::Degraded,
            "controller found; MMIO command engine pending",
        );
        for controller in &controllers {
            crate::serial_println!(
                "[AHCI] {:02x}:{:02x}.{} vendor={:04x} device={:04x} abar={:#x} mmio_ready={}",
                controller.pci.bus,
                controller.pci.slot,
                controller.pci.function,
                controller.pci.vendor_id,
                controller.pci.device_id,
                controller.abar,
                controller.mmio_ready
            );
        }
    }

    *CONTROLLERS.lock() = controllers;
}

fn controller_from_pci(device: PciDevice) -> Option<AhciController> {
    let (abar, prefetchable) = match pci::read_bar_info(device, AHCI_ABAR_INDEX) {
        PciBar::Memory32 {
            address,
            prefetchable,
        }
        | PciBar::Memory64 {
            address,
            prefetchable,
        } if address != 0 => (address, prefetchable),
        _ => return None,
    };
    Some(AhciController {
        pci: device,
        abar,
        prefetchable,
        mmio_ready: false,
    })
}

pub fn controller_count() -> usize {
    CONTROLLERS.lock().len()
}

pub fn write_to_buffer(out: &mut [u8]) -> usize {
    let controllers = CONTROLLERS.lock();
    let mut writer = BufferWriter::new(out);
    writer.write_str("AHCI controllers: ");
    writer.write_dec(controllers.len() as u64);
    writer.write_byte(b'\n');
    writer.write_str("BDF       VENDOR:DEVICE ABAR               STATE\n");
    for controller in controllers.iter() {
        writer.write_hex_u8(controller.pci.bus);
        writer.write_byte(b':');
        writer.write_hex_u8(controller.pci.slot);
        writer.write_byte(b'.');
        writer.write_dec(controller.pci.function as u64);
        writer.write_str("  ");
        writer.write_hex_u16(controller.pci.vendor_id);
        writer.write_byte(b':');
        writer.write_hex_u16(controller.pci.device_id);
        writer.write_str(" 0x");
        writer.write_hex_u64(controller.abar);
        writer.write_str(if controller.mmio_ready {
            " ready\n"
        } else {
            " discovery\n"
        });
    }
    writer.len()
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

    fn write_dec(&mut self, mut value: u64) {
        let mut digits = [0u8; 20];
        if value == 0 {
            self.write_byte(b'0');
            return;
        }
        let mut len = 0;
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

    fn write_hex_u64(&mut self, value: u64) {
        for shift in (0..16).rev() {
            self.write_hex_nibble((value >> (shift * 4)) as u8);
        }
    }

    fn write_hex_nibble(&mut self, value: u8) {
        let value = value & 0x0f;
        self.write_byte(if value < 10 {
            b'0' + value
        } else {
            b'a' + value - 10
        });
    }
}

#[cfg(test)]
mod tests {
    use super::BufferWriter;

    #[test_case]
    fn formats_full_width_abar() {
        let mut out = [0u8; 32];
        let mut writer = BufferWriter::new(&mut out);
        writer.write_hex_u64(0x1234_abcd);
        let len = writer.len();
        drop(writer);
        assert_eq!(&out[..len], b"000000001234abcd");
    }
}
