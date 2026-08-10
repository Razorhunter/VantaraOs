// USB Driver for Vantara OS
// Implements basic USB host controller support (UHCI)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UsbRequestType {
    Standard = 0x00,
    Class = 0x20,
    Vendor = 0x40,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UsbRequestRecipient {
    Device = 0x00,
    Interface = 0x01,
    Endpoint = 0x02,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UsbRequestDirection {
    HostToDevice = 0x00,
    DeviceToHost = 0x80,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UsbRequest {
    GetStatus = 0x00,
    ClearFeature = 0x01,
    SetFeature = 0x03,
    SetAddress = 0x05,
    GetDescriptor = 0x06,
    SetDescriptor = 0x07,
    GetConfiguration = 0x08,
    SetConfiguration = 0x09,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UsbDescriptorType {
    Device = 1,
    Configuration = 2,
    String = 3,
    Interface = 4,
    Endpoint = 5,
    DeviceQualifier = 6,
    OtherSpeedConfiguration = 7,
    InterfacePower = 8,
}

// USB Device Descriptor
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct UsbDeviceDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub usb_version: u16,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
    pub max_packet_size: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_version: u16,
    pub manufacturer_index: u8,
    pub product_index: u8,
    pub serial_number_index: u8,
    pub num_configurations: u8,
}

// USB Configuration Descriptor
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct UsbConfigurationDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub total_length: u16,
    pub num_interfaces: u8,
    pub configuration_value: u8,
    pub configuration_index: u8,
    pub attributes: u8,
    pub max_power: u8,
}

// USB Interface Descriptor
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct UsbInterfaceDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub interface_number: u8,
    pub alternate_setting: u8,
    pub num_endpoints: u8,
    pub interface_class: u8,
    pub interface_subclass: u8,
    pub interface_protocol: u8,
    pub interface_index: u8,
}

// USB Endpoint Descriptor
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct UsbEndpointDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub endpoint_address: u8,
    pub attributes: u8,
    pub max_packet_size: u16,
    pub interval: u8,
}

// UHCI Transfer Descriptor (TD)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct UhciTransferDescriptor {
    pub link_pointer: u32,
    pub control_status: u32,
    pub token: u32,
    pub buffer_pointer: u32,
}

impl UhciTransferDescriptor {
    pub fn new() -> Self {
        UhciTransferDescriptor {
            link_pointer: 1, // Invalid by default
            control_status: 0,
            token: 0,
            buffer_pointer: 0,
        }
    }

    pub fn set_active(&mut self) {
        self.control_status |= 1 << 23;
    }

    pub fn is_active(&self) -> bool {
        (self.control_status & (1 << 23)) != 0
    }

    pub fn get_status(&self) -> u8 {
        ((self.control_status >> 16) & 0xFF) as u8
    }
}

// UHCI Queue Head (QH)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct UhciQueueHead {
    pub link_pointer: u32,
    pub element_link_pointer: u32,
}

impl UhciQueueHead {
    pub fn new() -> Self {
        UhciQueueHead {
            link_pointer: 1, // Invalid by default
            element_link_pointer: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbDeviceClass {
    InterfaceSpecific = 0x00,
    Audio = 0x01,
    CommunicationAndCDC = 0x02,
    HID = 0x03,
    Physical = 0x05,
    Image = 0x06,
    Printer = 0x07,
    MassStorage = 0x08,
    Hub = 0x09,
    MiscellaneousDevice = 0xEF,
    VendorSpecific = 0xFF,
}

impl UsbDeviceClass {
    pub fn from_u8(val: u8) -> Self {
        match val {
            0x00 => UsbDeviceClass::InterfaceSpecific,
            0x01 => UsbDeviceClass::Audio,
            0x02 => UsbDeviceClass::CommunicationAndCDC,
            0x03 => UsbDeviceClass::HID,
            0x05 => UsbDeviceClass::Physical,
            0x06 => UsbDeviceClass::Image,
            0x07 => UsbDeviceClass::Printer,
            0x08 => UsbDeviceClass::MassStorage,
            0x09 => UsbDeviceClass::Hub,
            0xEF => UsbDeviceClass::MiscellaneousDevice,
            0xFF => UsbDeviceClass::VendorSpecific,
            _ => UsbDeviceClass::VendorSpecific,
        }
    }
}

#[derive(Debug, Clone)]
pub struct UsbDevice {
    pub address: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_class: UsbDeviceClass,
    pub num_configurations: u8,
    pub max_packet_size: u8,
}

impl UsbDevice {
    pub fn from_descriptor(desc: &UsbDeviceDescriptor, address: u8) -> Self {
        UsbDevice {
            address,
            vendor_id: desc.vendor_id,
            product_id: desc.product_id,
            device_class: UsbDeviceClass::from_u8(desc.device_class),
            num_configurations: desc.num_configurations,
            max_packet_size: desc.max_packet_size,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum UsbSpeed {
    LowSpeed = 0,
    FullSpeed = 1,
}
