use crate::sync::PreemptMutex as Mutex;
use alloc::vec::Vec;
use lazy_static::lazy_static;
use x86_64::instructions::port::Port;

const CONFIG_ADDRESS: u16 = 0xcf8;
const CONFIG_DATA: u16 = 0xcfc;
const INVALID_VENDOR_ID: u16 = 0xffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciDevice {
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub header_type: u8,
}

lazy_static! {
    static ref DEVICES: Mutex<Vec<PciDevice>> = Mutex::new(Vec::new());
}

pub fn init() {
    let devices = scan_bus();
    crate::drivers::status::report(
        "pci",
        crate::drivers::status::DriverState::Ready,
        "device registry available",
    );
    crate::serial_println!("[PCI] Found {} PCI device(s)", devices.len());

    for device in &devices {
        crate::serial_println!(
            "[PCI] {:02x}:{:02x}.{} vendor={:04x} device={:04x} class={:02x}/{:02x}/{:02x}",
            device.bus,
            device.slot,
            device.function,
            device.vendor_id,
            device.device_id,
            device.class_code,
            device.subclass,
            device.prog_if
        );
    }

    *DEVICES.lock() = devices;
}

pub fn device_count() -> usize {
    DEVICES.lock().len()
}

pub fn devices_by_class(class_code: u8) -> Vec<PciDevice> {
    DEVICES
        .lock()
        .iter()
        .copied()
        .filter(|device| device.class_code == class_code)
        .collect()
}

pub fn write_devices_to_buffer(out: &mut [u8]) -> usize {
    let devices = DEVICES.lock();
    let mut writer = BufferWriter::new(out);

    writer.write_str("PCI devices: ");
    writer.write_dec(devices.len() as u64);
    writer.write_byte(b'\n');
    writer.write_str("BDF       VENDOR DEVICE CLASS\n");

    for device in devices.iter() {
        writer.write_hex_u8(device.bus);
        writer.write_byte(b':');
        writer.write_hex_u8(device.slot);
        writer.write_byte(b'.');
        writer.write_dec(device.function as u64);
        writer.write_str("  ");
        writer.write_hex_u16(device.vendor_id);
        writer.write_str("   ");
        writer.write_hex_u16(device.device_id);
        writer.write_str("   ");
        writer.write_hex_u8(device.class_code);
        writer.write_byte(b'/');
        writer.write_hex_u8(device.subclass);
        writer.write_byte(b'/');
        writer.write_hex_u8(device.prog_if);
        writer.write_byte(b'\n');
    }

    writer.len()
}

pub fn find_device_by_class(
    class_code: u8,
    subclass: u8,
    prog_if: Option<u8>,
) -> Option<PciDevice> {
    DEVICES.lock().iter().copied().find(|device| {
        device.class_code == class_code
            && device.subclass == subclass
            && prog_if.map_or(true, |expected| device.prog_if == expected)
    })
}

pub fn read_bar(device: PciDevice, bar_index: u8) -> u32 {
    if bar_index >= 6 {
        return 0;
    }

    read_config_u32(
        device.bus,
        device.slot,
        device.function,
        0x10 + bar_index * 4,
    )
}

pub fn scan_bus() -> Vec<PciDevice> {
    let mut devices = Vec::new();

    for bus in 0..=255 {
        for slot in 0..32 {
            scan_slot(bus, slot, &mut devices);
        }
    }

    devices
}

fn scan_slot(bus: u8, slot: u8, devices: &mut Vec<PciDevice>) {
    if read_vendor_id(bus, slot, 0) == INVALID_VENDOR_ID {
        return;
    }

    scan_function(bus, slot, 0, devices);

    if read_header_type(bus, slot, 0) & 0x80 != 0 {
        for function in 1..8 {
            if read_vendor_id(bus, slot, function) != INVALID_VENDOR_ID {
                scan_function(bus, slot, function, devices);
            }
        }
    }
}

fn scan_function(bus: u8, slot: u8, function: u8, devices: &mut Vec<PciDevice>) {
    let vendor_id = read_vendor_id(bus, slot, function);
    if vendor_id == INVALID_VENDOR_ID {
        return;
    }

    let id = read_config_u32(bus, slot, function, 0x00);
    let class = read_config_u32(bus, slot, function, 0x08);

    devices.push(PciDevice {
        bus,
        slot,
        function,
        vendor_id,
        device_id: (id >> 16) as u16,
        class_code: (class >> 24) as u8,
        subclass: (class >> 16) as u8,
        prog_if: (class >> 8) as u8,
        header_type: read_header_type(bus, slot, function),
    });
}

fn read_vendor_id(bus: u8, slot: u8, function: u8) -> u16 {
    read_config_u32(bus, slot, function, 0x00) as u16
}

fn read_header_type(bus: u8, slot: u8, function: u8) -> u8 {
    (read_config_u32(bus, slot, function, 0x0c) >> 16) as u8
}

fn read_config_u32(bus: u8, slot: u8, function: u8, offset: u8) -> u32 {
    let address = 0x8000_0000u32
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((function as u32) << 8)
        | ((offset as u32) & 0xfc);

    unsafe {
        let mut address_port = Port::<u32>::new(CONFIG_ADDRESS);
        let mut data_port = Port::<u32>::new(CONFIG_DATA);

        address_port.write(address);
        data_port.read()
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

    fn write_str(&mut self, text: &str) {
        for byte in text.bytes() {
            self.write_byte(byte);
        }
    }

    fn write_byte(&mut self, byte: u8) {
        if self.len < self.out.len() {
            self.out[self.len] = byte;
            self.len += 1;
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
        self.write_hex_nibble(value & 0x0f);
    }

    fn write_hex_u16(&mut self, value: u16) {
        self.write_hex_u8((value >> 8) as u8);
        self.write_hex_u8(value as u8);
    }

    fn write_hex_nibble(&mut self, nibble: u8) {
        let digit = match nibble & 0x0f {
            value @ 0..=9 => b'0' + value,
            value => b'a' + (value - 10),
        };
        self.write_byte(digit);
    }
}

#[cfg(test)]
mod tests {
    use super::BufferWriter;

    #[test_case]
    fn formats_pci_identifiers() {
        let mut buffer = [0u8; 32];
        let mut writer = BufferWriter::new(&mut buffer);
        writer.write_hex_u8(0x0a);
        writer.write_byte(b':');
        writer.write_hex_u16(0x8086);
        writer.write_byte(b'/');
        writer.write_dec(7);

        let len = writer.len();
        drop(writer);
        assert_eq!(&buffer[..len], b"0a:8086/7");
    }
}
