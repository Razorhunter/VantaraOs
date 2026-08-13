use crate::sync::PreemptMutex as Mutex;

use super::pci::{self, PciBar, PciDevice};
use x86_64::{
    VirtAddr,
    structures::paging::{FrameAllocator, PageSize, Size4KiB},
};

const VIRTIO_VENDOR_ID: u16 = 0x1af4;
const VIRTIO_GPU_DEVICE_ID: u16 = 0x1050;
const PCI_STATUS_CAPABILITIES: u32 = 1 << 20;
const PCI_CAP_ID_VENDOR_SPECIFIC: u8 = 0x09;
const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
const VIRTIO_PCI_CAP_ISR_CFG: u8 = 3;
const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;
const VIRTIO_F_VERSION_1: u64 = 1 << 32;
const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;
const STATUS_FEATURES_OK: u8 = 8;
const CONTROL_QUEUE_INDEX: u16 = 0;
const MAX_CONTROL_QUEUE_SIZE: u16 = 256;
const COMMON_CFG_MIN_LENGTH: u32 = 56;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioPciCapability {
    pub bar: u8,
    pub offset: u32,
    pub length: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioGpuInfo {
    pub detected: bool,
    pub pci: Option<PciDevice>,
    pub bar0: PciBar,
    pub common: Option<VirtioPciCapability>,
    pub notify: Option<VirtioPciCapability>,
    pub isr: Option<VirtioPciCapability>,
    pub device: Option<VirtioPciCapability>,
    pub capability_count: u8,
    pub device_features: u64,
    pub negotiated_features: u64,
    pub features_ok: bool,
    pub control_queue_size: u16,
    pub control_queue_enabled: bool,
    pub driver_ok: bool,
    pub descriptor_frame: u64,
    pub available_frame: u64,
    pub used_frame: u64,
}

impl VirtioGpuInfo {
    const fn empty() -> Self {
        Self {
            detected: false,
            pci: None,
            bar0: PciBar::Unused,
            common: None,
            notify: None,
            isr: None,
            device: None,
            capability_count: 0,
            device_features: 0,
            negotiated_features: 0,
            features_ok: false,
            control_queue_size: 0,
            control_queue_enabled: false,
            driver_ok: false,
            descriptor_frame: 0,
            available_frame: 0,
            used_frame: 0,
        }
    }

    fn transport_ready(self) -> bool {
        self.common.is_some()
            && self.notify.is_some()
            && self.isr.is_some()
            && self.device.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransportError {
    MissingCommonConfig,
    InvalidCommonConfig,
    InvalidBar,
    MissingVersionOne,
    FeaturesRejected,
    MissingControlQueue,
    UnsupportedQueueSize,
    FrameAllocationFailed,
    QueueEnableFailed,
}

#[derive(Clone, Copy)]
struct MmioRegion {
    base: usize,
    length: usize,
}

impl MmioRegion {
    fn contains(self, offset: usize, size: usize) -> bool {
        offset
            .checked_add(size)
            .is_some_and(|end| end <= self.length)
    }

    fn read_u8(self, offset: usize) -> Option<u8> {
        if !self.contains(offset, 1) {
            return None;
        }
        // SAFETY: the capability bounds were validated and the boot physical
        // mapping keeps this device MMIO address mapped for the kernel.
        Some(unsafe { (self.base as *const u8).add(offset).read_volatile() })
    }

    fn read_u16(self, offset: usize) -> Option<u16> {
        if !self.contains(offset, 2) || (self.base + offset) % 2 != 0 {
            return None;
        }
        // SAFETY: bounds and alignment are checked above; register access is volatile.
        Some(unsafe { (self.base as *const u16).add(offset / 2).read_volatile() })
    }

    fn read_u32(self, offset: usize) -> Option<u32> {
        if !self.contains(offset, 4) || (self.base + offset) % 4 != 0 {
            return None;
        }
        // SAFETY: bounds and alignment are checked above; register access is volatile.
        Some(unsafe { (self.base as *const u32).add(offset / 4).read_volatile() })
    }

    fn write_u8(self, offset: usize, value: u8) -> bool {
        if !self.contains(offset, 1) {
            return false;
        }
        // SAFETY: the validated MMIO register is writable by the owning driver.
        unsafe { (self.base as *mut u8).add(offset).write_volatile(value) };
        true
    }

    fn write_u16(self, offset: usize, value: u16) -> bool {
        if !self.contains(offset, 2) || (self.base + offset) % 2 != 0 {
            return false;
        }
        // SAFETY: bounds and alignment are checked above; register access is volatile.
        unsafe {
            (self.base as *mut u16)
                .add(offset / 2)
                .write_volatile(value)
        };
        true
    }

    fn write_u32(self, offset: usize, value: u32) -> bool {
        if !self.contains(offset, 4) || (self.base + offset) % 4 != 0 {
            return false;
        }
        // SAFETY: bounds and alignment are checked above; register access is volatile.
        unsafe {
            (self.base as *mut u32)
                .add(offset / 4)
                .write_volatile(value)
        };
        true
    }

    fn write_u64(self, offset: usize, value: u64) -> bool {
        if !self.contains(offset, 8) || (self.base + offset) % 8 != 0 {
            return false;
        }
        // SAFETY: bounds and alignment are checked above; register access is volatile.
        unsafe {
            (self.base as *mut u64)
                .add(offset / 8)
                .write_volatile(value)
        };
        true
    }
}

static INFO: Mutex<VirtioGpuInfo> = Mutex::new(VirtioGpuInfo::empty());

pub fn init(physical_memory_offset: VirtAddr, frame_allocator: &mut impl FrameAllocator<Size4KiB>) {
    let Some(device) = pci::devices().into_iter().find(is_virtio_gpu) else {
        crate::drivers::status::report(
            "virtio-gpu",
            crate::drivers::status::DriverState::Missing,
            "PCI function not detected",
        );
        crate::serial_println!("[VIRTIO-GPU] no device detected");
        return;
    };

    let mut info = VirtioGpuInfo {
        detected: true,
        pci: Some(device),
        bar0: pci::read_bar_info(device, 0),
        ..VirtioGpuInfo::empty()
    };
    discover_capabilities(device, &mut info);
    let transport_result =
        initialize_transport(device, physical_memory_offset, frame_allocator, &mut info);
    let ready = transport_result.is_ok();
    crate::drivers::status::report(
        "virtio-gpu",
        if ready {
            crate::drivers::status::DriverState::Ready
        } else {
            crate::drivers::status::DriverState::Degraded
        },
        if ready {
            "VERSION_1 negotiated; control virtqueue enabled"
        } else {
            "device found; modern PCI capabilities incomplete"
        },
    );
    crate::serial_println!(
        "[VIRTIO-GPU] {:02x}:{:02x}.{} vendor={:04x} device={:04x} bar0={:?} capabilities={} common={} notify={} isr={} device_cfg={} transport_ready={} version1={} features_ok={} queue_size={} queue_enabled={} driver_ok={} init={:?}",
        device.bus,
        device.slot,
        device.function,
        device.vendor_id,
        device.device_id,
        info.bar0,
        info.capability_count,
        info.common.is_some(),
        info.notify.is_some(),
        info.isr.is_some(),
        info.device.is_some(),
        info.transport_ready(),
        info.negotiated_features & VIRTIO_F_VERSION_1 != 0,
        info.features_ok,
        info.control_queue_size,
        info.control_queue_enabled,
        info.driver_ok,
        transport_result
    );
    *INFO.lock() = info;
}

fn initialize_transport(
    pci_device: PciDevice,
    physical_memory_offset: VirtAddr,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    info: &mut VirtioGpuInfo,
) -> Result<(), TransportError> {
    let common_cap = info.common.ok_or(TransportError::MissingCommonConfig)?;
    if common_cap.length < COMMON_CFG_MIN_LENGTH {
        return Err(TransportError::InvalidCommonConfig);
    }
    let common = capability_region(pci_device, common_cap, physical_memory_offset)?;
    pci::enable_memory_and_bus_master(pci_device);

    if !common.write_u8(20, 0) || common.read_u8(20) != Some(0) {
        return Err(TransportError::InvalidCommonConfig);
    }
    common.write_u8(20, STATUS_ACKNOWLEDGE | STATUS_DRIVER);
    common.write_u32(0, 0);
    let low = common
        .read_u32(4)
        .ok_or(TransportError::InvalidCommonConfig)?;
    common.write_u32(0, 1);
    let high = common
        .read_u32(4)
        .ok_or(TransportError::InvalidCommonConfig)?;
    info.device_features = u64::from(low) | (u64::from(high) << 32);
    if info.device_features & VIRTIO_F_VERSION_1 == 0 {
        common.write_u8(20, STATUS_ACKNOWLEDGE | STATUS_DRIVER | 0x80);
        return Err(TransportError::MissingVersionOne);
    }

    info.negotiated_features = VIRTIO_F_VERSION_1;
    common.write_u32(8, 0);
    common.write_u32(12, 0);
    common.write_u32(8, 1);
    common.write_u32(12, 1);
    common.write_u8(20, STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK);
    let status = common
        .read_u8(20)
        .ok_or(TransportError::InvalidCommonConfig)?;
    if status & STATUS_FEATURES_OK == 0 {
        return Err(TransportError::FeaturesRejected);
    }
    info.features_ok = true;

    common.write_u16(22, CONTROL_QUEUE_INDEX);
    let offered_size = common
        .read_u16(24)
        .ok_or(TransportError::InvalidCommonConfig)?;
    if offered_size == 0 {
        return Err(TransportError::MissingControlQueue);
    }
    let queue_size = offered_size.min(MAX_CONTROL_QUEUE_SIZE);
    if usize::from(queue_size) * 16 > Size4KiB::SIZE as usize
        || 6 + usize::from(queue_size) * 2 > Size4KiB::SIZE as usize
        || 6 + usize::from(queue_size) * 8 > Size4KiB::SIZE as usize
    {
        return Err(TransportError::UnsupportedQueueSize);
    }
    let descriptor = allocate_zeroed_frame(physical_memory_offset, frame_allocator)?;
    let available = allocate_zeroed_frame(physical_memory_offset, frame_allocator)?;
    let used = allocate_zeroed_frame(physical_memory_offset, frame_allocator)?;

    common.write_u16(24, queue_size);
    common.write_u16(26, u16::MAX);
    if !common.write_u64(32, descriptor)
        || !common.write_u64(40, available)
        || !common.write_u64(48, used)
        || !common.write_u16(28, 1)
        || common.read_u16(28) != Some(1)
    {
        return Err(TransportError::QueueEnableFailed);
    }
    info.control_queue_size = queue_size;
    info.control_queue_enabled = true;
    info.descriptor_frame = descriptor;
    info.available_frame = available;
    info.used_frame = used;

    common.write_u8(
        20,
        STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
    );
    info.driver_ok = common
        .read_u8(20)
        .is_some_and(|value| value & STATUS_DRIVER_OK != 0);
    if !info.driver_ok {
        return Err(TransportError::QueueEnableFailed);
    }
    Ok(())
}

fn capability_region(
    device: PciDevice,
    capability: VirtioPciCapability,
    physical_memory_offset: VirtAddr,
) -> Result<MmioRegion, TransportError> {
    let bar_address = match pci::read_bar_info(device, capability.bar) {
        PciBar::Memory32 { address, .. } | PciBar::Memory64 { address, .. } => address,
        _ => return Err(TransportError::InvalidBar),
    };
    let physical = bar_address
        .checked_add(u64::from(capability.offset))
        .ok_or(TransportError::InvalidBar)?;
    let virtual_address = physical_memory_offset
        .as_u64()
        .checked_add(physical)
        .ok_or(TransportError::InvalidBar)?;
    Ok(MmioRegion {
        base: usize::try_from(virtual_address).map_err(|_| TransportError::InvalidBar)?,
        length: capability.length as usize,
    })
}

fn allocate_zeroed_frame(
    physical_memory_offset: VirtAddr,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<u64, TransportError> {
    let frame = frame_allocator
        .allocate_frame()
        .ok_or(TransportError::FrameAllocationFailed)?;
    let physical = frame.start_address().as_u64();
    let destination = (physical_memory_offset + physical).as_mut_ptr::<u8>();
    // SAFETY: the frame was uniquely allocated above and the boot physical
    // mapping covers its complete 4 KiB range.
    unsafe { core::ptr::write_bytes(destination, 0, Size4KiB::SIZE as usize) };
    Ok(physical)
}

fn is_virtio_gpu(device: &PciDevice) -> bool {
    device.vendor_id == VIRTIO_VENDOR_ID
        && device.device_id == VIRTIO_GPU_DEVICE_ID
        && device.class_code == 0x03
}

fn discover_capabilities(device: PciDevice, info: &mut VirtioGpuInfo) {
    let command_status = pci::read_config_u32(device.bus, device.slot, device.function, 0x04);
    if command_status & PCI_STATUS_CAPABILITIES == 0 {
        return;
    }
    let mut pointer = pci::read_config_u8(device, 0x34) & !0x03;
    for _ in 0..48 {
        if pointer < 0x40 || pointer > 0xfc {
            break;
        }
        let id = pci::read_config_u8(device, pointer);
        let next = pci::read_config_u8(device, pointer.saturating_add(1)) & !0x03;
        if id == PCI_CAP_ID_VENDOR_SPECIFIC {
            let cap_len = pci::read_config_u8(device, pointer.saturating_add(2));
            if cap_len >= 16 && pointer <= 0xec {
                let cfg_type = pci::read_config_u8(device, pointer + 3);
                let capability = VirtioPciCapability {
                    bar: pci::read_config_u8(device, pointer + 4),
                    offset: pci::read_config_u32(
                        device.bus,
                        device.slot,
                        device.function,
                        pointer + 8,
                    ),
                    length: pci::read_config_u32(
                        device.bus,
                        device.slot,
                        device.function,
                        pointer + 12,
                    ),
                };
                info.capability_count = info.capability_count.saturating_add(1);
                match cfg_type {
                    VIRTIO_PCI_CAP_COMMON_CFG => info.common = Some(capability),
                    VIRTIO_PCI_CAP_NOTIFY_CFG => info.notify = Some(capability),
                    VIRTIO_PCI_CAP_ISR_CFG => info.isr = Some(capability),
                    VIRTIO_PCI_CAP_DEVICE_CFG => info.device = Some(capability),
                    _ => {}
                }
            }
        }
        if next == 0 || next == pointer {
            break;
        }
        pointer = next;
    }
}

pub fn write_to_buffer(out: &mut [u8]) -> usize {
    let info = *INFO.lock();
    let mut writer = Writer { out, len: 0 };
    writer.text("detected=");
    writer.bool(info.detected);
    writer.text(" transport-ready=");
    writer.bool(info.transport_ready());
    writer.text(" capabilities=");
    writer.dec(u64::from(info.capability_count));
    writer.text(" common=");
    writer.bool(info.common.is_some());
    writer.text(" notify=");
    writer.bool(info.notify.is_some());
    writer.text(" isr=");
    writer.bool(info.isr.is_some());
    writer.text(" device-config=");
    writer.bool(info.device.is_some());
    writer.text(" version1=");
    writer.bool(info.negotiated_features & VIRTIO_F_VERSION_1 != 0);
    writer.text(" features-ok=");
    writer.bool(info.features_ok);
    writer.text(" queue-size=");
    writer.dec(u64::from(info.control_queue_size));
    writer.text(" queue-enabled=");
    writer.bool(info.control_queue_enabled);
    writer.text(" driver-ok=");
    writer.bool(info.driver_ok);
    writer.byte(b'\n');
    writer.len
}

struct Writer<'a> {
    out: &'a mut [u8],
    len: usize,
}

impl Writer<'_> {
    fn byte(&mut self, byte: u8) {
        if self.len < self.out.len() {
            self.out[self.len] = byte;
            self.len += 1;
        }
    }
    fn text(&mut self, text: &str) {
        for byte in text.bytes() {
            self.byte(byte);
        }
    }
    fn bool(&mut self, value: bool) {
        self.text(if value { "true" } else { "false" });
    }
    fn dec(&mut self, mut value: u64) {
        let mut digits = [0u8; 20];
        let mut start = digits.len();
        loop {
            start -= 1;
            digits[start] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        for byte in &digits[start..] {
            self.byte(*byte);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PciDevice, is_virtio_gpu};

    #[test_case]
    fn identifies_only_virtio_gpu_display_function() {
        let mut device = PciDevice {
            bus: 0,
            slot: 1,
            function: 0,
            vendor_id: 0x1af4,
            device_id: 0x1050,
            class_code: 0x03,
            subclass: 0,
            prog_if: 0,
            header_type: 0,
        };
        assert!(is_virtio_gpu(&device));
        device.device_id = 0x1041;
        assert!(!is_virtio_gpu(&device));
    }
}
