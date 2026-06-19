#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbDescriptorError {
    TooShort,
    InvalidLength,
    UnexpectedType,
    Truncated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceDescriptor {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigurationDescriptor {
    pub total_length: u16,
    pub num_interfaces: u8,
    pub configuration_value: u8,
    pub attributes: u8,
    pub max_power_ma: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceDescriptor {
    pub interface_number: u8,
    pub alternate_setting: u8,
    pub num_endpoints: u8,
    pub interface_class: u8,
    pub interface_subclass: u8,
    pub interface_protocol: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointDescriptor {
    pub address: u8,
    pub attributes: u8,
    pub max_packet_size: u16,
    pub interval: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedDescriptor<'a> {
    Configuration(ConfigurationDescriptor),
    Interface(InterfaceDescriptor),
    Endpoint(EndpointDescriptor),
    Unknown {
        descriptor_type: u8,
        bytes: &'a [u8],
    },
}

pub struct DescriptorIter<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> DescriptorIter<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl<'a> Iterator for DescriptorIter<'a> {
    type Item = Result<ParsedDescriptor<'a>, UsbDescriptorError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset == self.bytes.len() {
            return None;
        }
        if self.bytes.len() - self.offset < 2 {
            self.offset = self.bytes.len();
            return Some(Err(UsbDescriptorError::TooShort));
        }

        let length = self.bytes[self.offset] as usize;
        if length < 2 {
            self.offset = self.bytes.len();
            return Some(Err(UsbDescriptorError::InvalidLength));
        }

        let end = match self.offset.checked_add(length) {
            Some(end) if end <= self.bytes.len() => end,
            _ => {
                self.offset = self.bytes.len();
                return Some(Err(UsbDescriptorError::Truncated));
            }
        };
        let descriptor = &self.bytes[self.offset..end];
        self.offset = end;

        Some(parse_descriptor(descriptor))
    }
}

pub fn parse_device_descriptor(bytes: &[u8]) -> Result<DeviceDescriptor, UsbDescriptorError> {
    require_descriptor(bytes, 18, 1)?;
    Ok(DeviceDescriptor {
        usb_version: read_u16(bytes, 2),
        device_class: bytes[4],
        device_subclass: bytes[5],
        device_protocol: bytes[6],
        max_packet_size: bytes[7],
        vendor_id: read_u16(bytes, 8),
        product_id: read_u16(bytes, 10),
        device_version: read_u16(bytes, 12),
        manufacturer_index: bytes[14],
        product_index: bytes[15],
        serial_number_index: bytes[16],
        num_configurations: bytes[17],
    })
}

fn parse_descriptor(bytes: &[u8]) -> Result<ParsedDescriptor<'_>, UsbDescriptorError> {
    match bytes[1] {
        2 => {
            require_descriptor(bytes, 9, 2)?;
            Ok(ParsedDescriptor::Configuration(ConfigurationDescriptor {
                total_length: read_u16(bytes, 2),
                num_interfaces: bytes[4],
                configuration_value: bytes[5],
                attributes: bytes[7],
                max_power_ma: bytes[8] as u16 * 2,
            }))
        }
        4 => {
            require_descriptor(bytes, 9, 4)?;
            Ok(ParsedDescriptor::Interface(InterfaceDescriptor {
                interface_number: bytes[2],
                alternate_setting: bytes[3],
                num_endpoints: bytes[4],
                interface_class: bytes[5],
                interface_subclass: bytes[6],
                interface_protocol: bytes[7],
            }))
        }
        5 => {
            require_descriptor(bytes, 7, 5)?;
            Ok(ParsedDescriptor::Endpoint(EndpointDescriptor {
                address: bytes[2],
                attributes: bytes[3],
                max_packet_size: read_u16(bytes, 4),
                interval: bytes[6],
            }))
        }
        descriptor_type => Ok(ParsedDescriptor::Unknown {
            descriptor_type,
            bytes,
        }),
    }
}

fn require_descriptor(
    bytes: &[u8],
    minimum_length: usize,
    expected_type: u8,
) -> Result<(), UsbDescriptorError> {
    if bytes.len() < minimum_length {
        return Err(UsbDescriptorError::TooShort);
    }
    if bytes[0] as usize > bytes.len() || (bytes[0] as usize) < minimum_length {
        return Err(UsbDescriptorError::InvalidLength);
    }
    if bytes[1] != expected_type {
        return Err(UsbDescriptorError::UnexpectedType);
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

#[cfg(test)]
mod tests {
    use super::{DescriptorIter, ParsedDescriptor, UsbDescriptorError, parse_device_descriptor};

    #[test_case]
    fn parses_usb_device_identity() {
        let descriptor = [
            18, 1, 0x00, 0x02, 0, 0, 0, 8, 0x6a, 0x0b, 0x46, 0x00, 0x00, 0x01, 1, 2, 3, 1,
        ];
        let parsed = parse_device_descriptor(&descriptor).unwrap();

        assert_eq!(parsed.usb_version, 0x0200);
        assert_eq!(parsed.vendor_id, 0x0b6a);
        assert_eq!(parsed.product_id, 0x0046);
        assert_eq!(parsed.max_packet_size, 8);
    }

    #[test_case]
    fn walks_hid_configuration_descriptors() {
        let descriptors = [
            9, 2, 25, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 1, 3, 1, 1, 0, 7, 5, 0x81, 3, 8, 0, 10,
        ];
        let mut iter = DescriptorIter::new(&descriptors);

        assert!(matches!(
            iter.next().unwrap().unwrap(),
            ParsedDescriptor::Configuration(config)
                if config.total_length == 25 && config.max_power_ma == 100
        ));
        assert!(matches!(
            iter.next().unwrap().unwrap(),
            ParsedDescriptor::Interface(interface)
                if interface.interface_class == 3 && interface.interface_protocol == 1
        ));
        assert!(matches!(
            iter.next().unwrap().unwrap(),
            ParsedDescriptor::Endpoint(endpoint)
                if endpoint.address == 0x81 && endpoint.max_packet_size == 8
        ));
        assert!(iter.next().is_none());
    }

    #[test_case]
    fn rejects_truncated_descriptor() {
        let mut iter = DescriptorIter::new(&[7, 5, 0x81]);
        assert_eq!(iter.next().unwrap(), Err(UsbDescriptorError::Truncated));
    }
}
