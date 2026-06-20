// UHCI (Universal Host Controller Interface) implementation
use crate::drivers::pci;
use crate::drivers::usb_descriptor::{
    DescriptorIter, ParsedDescriptor, UsbDescriptorError, parse_device_descriptor,
};
use crate::drivers::usb_structs::*;
use crate::sync::PreemptMutex as Mutex;
use alloc::vec::Vec;
use lazy_static::lazy_static;
use x86_64::instructions::port::Port;

pub const UHCI_MAX_DEVICES: usize = 127;

#[repr(u8)]
pub enum UhciCommand {
    Run = 0x01,
    HostControllerReset = 0x02,
    GlobalReset = 0x04,
    SuspendResume = 0x08,
    ForceGlobalResume = 0x10,
    SoftwareDebug = 0x20,
    ConfigureFlag = 0x40,
    MaximumPacket = 0x80,
}

#[repr(u8)]
pub enum UhciStatus {
    Interrupt = 0x01,
    ErrorInterrupt = 0x02,
    ResumeDetect = 0x04,
    HostSystemError = 0x08,
    HostControllerProcessError = 0x10,
    HcHalted = 0x20,
}

pub struct UhciController {
    iobase: u16,
    devices: Vec<UsbDevice>,
}

impl UhciController {
    pub fn new(iobase: u16) -> Self {
        UhciController {
            iobase,
            devices: Vec::new(),
        }
    }

    pub fn init(&mut self) -> Result<(), &'static str> {
        crate::serial_println!(
            "[USB] Initializing UHCI controller at I/O base 0x{:x}",
            self.iobase
        );

        // Reset the controller
        self.reset()?;

        // Enable the controller
        self.enable()?;

        crate::serial_println!("[USB] UHCI controller initialized successfully");
        Ok(())
    }

    fn reset(&mut self) -> Result<(), &'static str> {
        // Reset UHCI controller
        let mut command_port = Port::new(self.iobase);
        unsafe {
            command_port.write(UhciCommand::HostControllerReset as u16);
        }

        // Wait for reset to complete (max 10ms)
        for _ in 0..1000 {
            unsafe {
                let status: u16 = command_port.read();
                if (status & (UhciCommand::HostControllerReset as u16)) == 0 {
                    return Ok(());
                }
            }
            // Small delay
            for _ in 0..100 {
                core::hint::spin_loop();
            }
        }

        Err("UHCI reset timeout")
    }

    fn enable(&mut self) -> Result<(), &'static str> {
        let mut command_port = Port::new(self.iobase);
        unsafe {
            command_port.write(UhciCommand::Run as u16);
        }
        crate::serial_println!("[USB] UHCI controller enabled");
        Ok(())
    }

    pub fn get_port_status(&self, port: u8) -> u16 {
        if port >= 2 {
            return 0;
        }
        let port_offset = 0x10 + (port as u16) * 2;
        let mut port_reg = Port::new(self.iobase + port_offset);
        unsafe { port_reg.read() }
    }

    pub fn detect_devices(&mut self) -> Result<(), &'static str> {
        crate::serial_println!("[USB] Scanning for connected devices...");

        for port in 0..2 {
            let status = self.get_port_status(port);

            // Check if device is connected (bit 0: CurrentConnectStatus)
            if (status & 0x0001) != 0 {
                crate::serial_println!("[USB] Device detected on port {}", port);
                // Device connected but enumeration requires more complex setup
            }
        }

        Ok(())
    }

    pub fn enumerate_device(&mut self, address: u8) -> Result<UsbDevice, &'static str> {
        crate::serial_println!("[USB] Enumerating device at address {}", address);

        // In a full implementation, we would:
        // 1. Send GET_DESCRIPTOR request to device
        // 2. Parse device descriptor
        // 3. Send SET_ADDRESS command
        // 4. Get configuration descriptors
        // 5. Set configuration

        // For now, return a placeholder
        let device = UsbDevice {
            address,
            vendor_id: 0x0000,
            product_id: 0x0000,
            device_class: UsbDeviceClass::InterfaceSpecific,
            num_configurations: 1,
            max_packet_size: 8,
        };

        self.devices.push(device.clone());
        Ok(device)
    }

    pub fn register_descriptor_set(
        &mut self,
        address: u8,
        device_bytes: &[u8],
        configuration_bytes: &[u8],
    ) -> Result<UsbDevice, UsbDescriptorError> {
        let descriptor = parse_device_descriptor(device_bytes)?;
        let device = UsbDevice {
            address,
            vendor_id: descriptor.vendor_id,
            product_id: descriptor.product_id,
            device_class: UsbDeviceClass::from_u8(descriptor.device_class),
            num_configurations: descriptor.num_configurations,
            max_packet_size: descriptor.max_packet_size,
        };

        let mut interfaces = 0usize;
        let mut endpoints = 0usize;
        for parsed in DescriptorIter::new(configuration_bytes) {
            match parsed? {
                ParsedDescriptor::Interface(_) => interfaces += 1,
                ParsedDescriptor::Endpoint(_) => endpoints += 1,
                ParsedDescriptor::Configuration(_) | ParsedDescriptor::Unknown { .. } => {}
            }
        }

        crate::serial_println!(
            "[USB] descriptor parsed addr={} vid={:04x} pid={:04x} class={:?} configs={} interfaces={} endpoints={}",
            device.address,
            device.vendor_id,
            device.product_id,
            device.device_class,
            device.num_configurations,
            interfaces,
            endpoints
        );
        self.devices.push(device.clone());
        Ok(device)
    }

    pub fn get_devices(&self) -> &[UsbDevice] {
        &self.devices
    }
}

lazy_static! {
    pub static ref UHCI_CONTROLLER: Mutex<Option<UhciController>> = Mutex::new(None);
}

pub fn init_usb() {
    crate::serial_println!("[USB] Initializing USB subsystem...");

    let Some(iobase) = find_uhci_iobase() else {
        crate::drivers::status::report(
            "usb-uhci",
            crate::drivers::status::DriverState::Missing,
            "controller not found",
        );
        crate::serial_println!("[USB] No UHCI controller found through PCI");
        return;
    };

    let mut controller = UhciController::new(iobase);
    match controller.init() {
        Ok(_) => {
            let _ = controller.detect_devices();
            *UHCI_CONTROLLER.lock() = Some(controller);
            crate::drivers::status::report(
                "usb-uhci",
                crate::drivers::status::DriverState::Ready,
                "controller initialized",
            );
            crate::serial_println!("[USB] USB subsystem initialized");
        }
        Err(e) => {
            crate::drivers::status::report(
                "usb-uhci",
                crate::drivers::status::DriverState::Error,
                e,
            );
            crate::serial_println!("[USB] Failed to initialize UHCI: {}", e);
        }
    }
}

fn find_uhci_iobase() -> Option<u16> {
    let device = pci::find_device_by_class(0x0c, 0x03, Some(0x00))?;
    let bar = pci::read_bar(device, 4);

    if bar & 0x1 == 0 {
        crate::serial_println!("[USB] UHCI BAR4 is not an I/O BAR");
        return None;
    }

    let iobase = (bar & 0xfffc) as u16;
    if iobase == 0 {
        return None;
    }

    crate::serial_println!(
        "[USB] UHCI controller found at PCI {:02x}:{:02x}.{} I/O base 0x{:x}",
        device.bus,
        device.slot,
        device.function,
        iobase
    );
    Some(iobase)
}

pub fn get_devices() -> usize {
    UHCI_CONTROLLER
        .lock()
        .as_ref()
        .map(|c| c.get_devices().len())
        .unwrap_or(0)
}
