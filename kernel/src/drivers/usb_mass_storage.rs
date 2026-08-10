use crate::drivers::usb_descriptor::{
    DescriptorIter, EndpointDescriptor, InterfaceDescriptor, ParsedDescriptor, UsbDescriptorError,
};

pub const MASS_STORAGE_CLASS: u8 = 0x08;
pub const SCSI_TRANSPARENT_SUBCLASS: u8 = 0x06;
pub const BULK_ONLY_TRANSPORT_PROTOCOL: u8 = 0x50;
const ENDPOINT_DIRECTION_IN: u8 = 0x80;
const ENDPOINT_TRANSFER_TYPE_MASK: u8 = 0x03;
const ENDPOINT_TRANSFER_BULK: u8 = 0x02;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbMassStorageInterface {
    pub interface_number: u8,
    pub alternate_setting: u8,
    pub bulk_in_endpoint: u8,
    pub bulk_in_max_packet_size: u16,
    pub bulk_out_endpoint: u8,
    pub bulk_out_max_packet_size: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbMassStorageDetectionError {
    Descriptor(UsbDescriptorError),
    MissingBulkEndpoints,
}

impl From<UsbDescriptorError> for UsbMassStorageDetectionError {
    fn from(error: UsbDescriptorError) -> Self {
        Self::Descriptor(error)
    }
}

pub fn detect_interface(
    configuration_bytes: &[u8],
) -> Result<Option<UsbMassStorageInterface>, UsbMassStorageDetectionError> {
    let mut candidate = None;
    let mut bulk_in = None;
    let mut bulk_out = None;

    for descriptor in DescriptorIter::new(configuration_bytes) {
        match descriptor? {
            ParsedDescriptor::Interface(interface) => {
                if let Some(profile) = complete_profile(candidate, bulk_in, bulk_out) {
                    return Ok(Some(profile));
                }
                if candidate.is_some() {
                    return Err(UsbMassStorageDetectionError::MissingBulkEndpoints);
                }

                candidate = is_supported_interface(interface).then_some(interface);
                bulk_in = None;
                bulk_out = None;
            }
            ParsedDescriptor::Endpoint(endpoint) if candidate.is_some() => {
                if !is_bulk_endpoint(endpoint) {
                    continue;
                }
                if endpoint.address & ENDPOINT_DIRECTION_IN != 0 {
                    bulk_in.get_or_insert(endpoint);
                } else {
                    bulk_out.get_or_insert(endpoint);
                }
            }
            ParsedDescriptor::Configuration(_)
            | ParsedDescriptor::Endpoint(_)
            | ParsedDescriptor::Unknown { .. } => {}
        }
    }

    if let Some(profile) = complete_profile(candidate, bulk_in, bulk_out) {
        Ok(Some(profile))
    } else if candidate.is_some() {
        Err(UsbMassStorageDetectionError::MissingBulkEndpoints)
    } else {
        Ok(None)
    }
}

fn is_supported_interface(interface: InterfaceDescriptor) -> bool {
    interface.interface_class == MASS_STORAGE_CLASS
        && interface.interface_subclass == SCSI_TRANSPARENT_SUBCLASS
        && interface.interface_protocol == BULK_ONLY_TRANSPORT_PROTOCOL
}

fn is_bulk_endpoint(endpoint: EndpointDescriptor) -> bool {
    endpoint.attributes & ENDPOINT_TRANSFER_TYPE_MASK == ENDPOINT_TRANSFER_BULK
}

fn complete_profile(
    interface: Option<InterfaceDescriptor>,
    bulk_in: Option<EndpointDescriptor>,
    bulk_out: Option<EndpointDescriptor>,
) -> Option<UsbMassStorageInterface> {
    let interface = interface?;
    let bulk_in = bulk_in?;
    let bulk_out = bulk_out?;
    Some(UsbMassStorageInterface {
        interface_number: interface.interface_number,
        alternate_setting: interface.alternate_setting,
        bulk_in_endpoint: bulk_in.address,
        bulk_in_max_packet_size: bulk_in.max_packet_size,
        bulk_out_endpoint: bulk_out.address,
        bulk_out_max_packet_size: bulk_out.max_packet_size,
    })
}

#[cfg(test)]
mod tests {
    use super::{UsbMassStorageDetectionError, detect_interface};

    #[test_case]
    fn detects_scsi_bulk_only_interface_and_endpoints() {
        let descriptors = [
            9, 2, 32, 0, 1, 1, 0, 0x80, 50, // configuration
            9, 4, 2, 0, 2, 0x08, 0x06, 0x50, 0, // interface
            7, 5, 0x02, 0x02, 64, 0, 0, // bulk OUT
            7, 5, 0x81, 0x02, 64, 0, 0, // bulk IN
        ];

        let profile = detect_interface(&descriptors).unwrap().unwrap();
        assert_eq!(profile.interface_number, 2);
        assert_eq!(profile.bulk_out_endpoint, 0x02);
        assert_eq!(profile.bulk_out_max_packet_size, 64);
        assert_eq!(profile.bulk_in_endpoint, 0x81);
        assert_eq!(profile.bulk_in_max_packet_size, 64);
    }

    #[test_case]
    fn ignores_non_mass_storage_interface() {
        let descriptors = [
            9, 2, 25, 0, 1, 1, 0, 0x80, 50, // configuration
            9, 4, 0, 0, 1, 0x03, 0x01, 0x01, 0, // HID keyboard
            7, 5, 0x81, 0x03, 8, 0, 10, // interrupt IN
        ];

        assert_eq!(detect_interface(&descriptors).unwrap(), None);
    }

    #[test_case]
    fn rejects_mass_storage_interface_without_bulk_out() {
        let descriptors = [
            9, 2, 25, 0, 1, 1, 0, 0x80, 50, // configuration
            9, 4, 0, 0, 2, 0x08, 0x06, 0x50, 0, // interface
            7, 5, 0x81, 0x02, 64, 0, 0, // bulk IN only
        ];

        assert_eq!(
            detect_interface(&descriptors),
            Err(UsbMassStorageDetectionError::MissingBulkEndpoints)
        );
    }

    #[test_case]
    fn rejects_interrupt_endpoints_for_bulk_only_transport() {
        let descriptors = [
            9, 2, 32, 0, 1, 1, 0, 0x80, 50, // configuration
            9, 4, 0, 0, 2, 0x08, 0x06, 0x50, 0, // interface
            7, 5, 0x02, 0x03, 64, 0, 0, // interrupt OUT
            7, 5, 0x81, 0x03, 64, 0, 0, // interrupt IN
        ];

        assert_eq!(
            detect_interface(&descriptors),
            Err(UsbMassStorageDetectionError::MissingBulkEndpoints)
        );
    }
}
