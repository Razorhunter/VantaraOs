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
const VIRTIO_GPU_CMD_GET_DISPLAY_INFO: u32 = 0x0100;
const VIRTIO_GPU_CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
const VIRTIO_GPU_CMD_SET_SCANOUT: u32 = 0x0103;
const VIRTIO_GPU_CMD_RESOURCE_FLUSH: u32 = 0x0104;
const VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
const VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const VIRTIO_GPU_RESP_OK_NODATA: u32 = 0x1100;
const VIRTIO_GPU_RESP_OK_DISPLAY_INFO: u32 = 0x1101;
const VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM: u32 = 2;
const PRIMARY_RESOURCE_ID: u32 = 1;
const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;
const GPU_RESPONSE_OFFSET: usize = 64;
const GPU_CTRL_HEADER_SIZE: u32 = 24;
const GPU_DISPLAY_INFO_SIZE: u32 = 24 + 16 * 24;
const COMMAND_POLL_LIMIT: usize = 10_000_000;

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
    pub notify_multiplier: u32,
    pub device_features: u64,
    pub negotiated_features: u64,
    pub features_ok: bool,
    pub control_queue_size: u16,
    pub control_queue_enabled: bool,
    pub driver_ok: bool,
    pub descriptor_frame: u64,
    pub available_frame: u64,
    pub used_frame: u64,
    pub command_frame: u64,
    pub commands_submitted: u64,
    pub commands_completed: u64,
    pub response_type: u32,
    pub scanout_count: u8,
    pub primary_enabled: bool,
    pub primary_width: u32,
    pub primary_height: u32,
    pub resource_created: bool,
    pub backing_attached: bool,
    pub transfer_complete: bool,
    pub flush_complete: bool,
    pub scanout_configured: bool,
    pub backing_frame: u64,
    pub backing_bytes: u64,
    pub physical_memory_offset: u64,
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
            notify_multiplier: 0,
            device_features: 0,
            negotiated_features: 0,
            features_ok: false,
            control_queue_size: 0,
            control_queue_enabled: false,
            driver_ok: false,
            descriptor_frame: 0,
            available_frame: 0,
            used_frame: 0,
            command_frame: 0,
            commands_submitted: 0,
            commands_completed: 0,
            response_type: 0,
            scanout_count: 0,
            primary_enabled: false,
            primary_width: 0,
            primary_height: 0,
            resource_created: false,
            backing_attached: false,
            transfer_complete: false,
            flush_complete: false,
            scanout_configured: false,
            backing_frame: 0,
            backing_bytes: 0,
            physical_memory_offset: 0,
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
    InvalidNotifyConfig,
    CommandTimeout,
    InvalidGpuResponse,
    InvalidDisplayMode,
    NonContiguousBacking,
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
        physical_memory_offset: physical_memory_offset.as_u64(),
        ..VirtioGpuInfo::empty()
    };
    discover_capabilities(device, &mut info);
    let command_result =
        initialize_transport(device, physical_memory_offset, frame_allocator, &mut info)
            .and_then(|()| {
                get_display_info(device, physical_memory_offset, frame_allocator, &mut info)
            })
            .and_then(|()| {
                initialize_primary_resource(
                    device,
                    physical_memory_offset,
                    frame_allocator,
                    &mut info,
                )
            });
    let ready = command_result.is_ok();
    crate::drivers::status::report(
        "virtio-gpu",
        if ready {
            crate::drivers::status::DriverState::Ready
        } else {
            crate::drivers::status::DriverState::Degraded
        },
        if ready {
            "VirtIO-GPU 2D resource attached and scanned out"
        } else {
            "device found; modern PCI capabilities incomplete"
        },
    );
    crate::serial_println!(
        "[VIRTIO-GPU] {:02x}:{:02x}.{} vendor={:04x} device={:04x} bar0={:?} capabilities={} common={} notify={} isr={} device_cfg={} transport_ready={} version1={} features_ok={} queue_size={} queue_enabled={} driver_ok={} commands={}/{} response={:#x} scanouts={} primary={}x{} enabled={} resource={} backing={} transfer={} flush={} scanout={} bytes={} init={:?}",
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
        info.commands_completed,
        info.commands_submitted,
        info.response_type,
        info.scanout_count,
        info.primary_width,
        info.primary_height,
        info.primary_enabled,
        info.resource_created,
        info.backing_attached,
        info.transfer_complete,
        info.flush_complete,
        info.scanout_configured,
        info.backing_bytes,
        command_result
    );
    *INFO.lock() = info;
}

pub fn info() -> VirtioGpuInfo {
    *INFO.lock()
}

pub fn present(buffer: &[u8]) -> bool {
    let info = info();
    present_region(
        buffer,
        0,
        0,
        info.primary_width as usize,
        info.primary_height as usize,
        info.primary_width as usize * 4,
    )
}

pub fn present_region(
    buffer: &[u8],
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    stride: usize,
) -> bool {
    let mut info = INFO.lock();
    if !info.scanout_configured
        || buffer.len() as u64 != info.backing_bytes
        || stride != info.primary_width as usize * 4
        || x.saturating_add(width) > info.primary_width as usize
        || y.saturating_add(height) > info.primary_height as usize
        || width == 0
        || height == 0
    {
        return false;
    }
    let Some(device) = info.pci else {
        return false;
    };
    let physical_memory_offset = VirtAddr::new(info.physical_memory_offset);
    let backing = (physical_memory_offset + info.backing_frame).as_mut_ptr::<u8>();
    let row_offset = x * 4;
    let row_bytes = width * 4;
    for row in y..y + height {
        let offset = row * stride + row_offset;
        // SAFETY: region validation bounds each source and destination row to
        // the equally sized compositor and GPU backing buffers.
        unsafe {
            core::ptr::copy_nonoverlapping(
                buffer.as_ptr().add(offset),
                backing.add(offset),
                row_bytes,
            )
        };
    }
    transfer_and_flush_region(
        device,
        physical_memory_offset,
        &mut info,
        x as u32,
        y as u32,
        width as u32,
        height as u32,
    )
    .is_ok()
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

#[repr(C)]
#[derive(Clone, Copy)]
struct VirtqDescriptor {
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
}

fn get_display_info(
    pci_device: PciDevice,
    physical_memory_offset: VirtAddr,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    info: &mut VirtioGpuInfo,
) -> Result<(), TransportError> {
    let common = capability_region(
        pci_device,
        info.common.ok_or(TransportError::MissingCommonConfig)?,
        physical_memory_offset,
    )?;
    let notify_cap = info.notify.ok_or(TransportError::InvalidNotifyConfig)?;
    if info.notify_multiplier == 0 {
        return Err(TransportError::InvalidNotifyConfig);
    }
    common.write_u16(22, CONTROL_QUEUE_INDEX);
    let queue_notify_off = common
        .read_u16(30)
        .ok_or(TransportError::InvalidCommonConfig)?;
    let notify_offset = u64::from(queue_notify_off)
        .checked_mul(u64::from(info.notify_multiplier))
        .ok_or(TransportError::InvalidNotifyConfig)?;
    if notify_offset + 2 > u64::from(notify_cap.length) {
        return Err(TransportError::InvalidNotifyConfig);
    }
    let notify = capability_region(pci_device, notify_cap, physical_memory_offset)?;

    let command = allocate_zeroed_frame(physical_memory_offset, frame_allocator)?;
    info.command_frame = command;
    let command_virtual = physical_memory_offset + command;
    // SAFETY: command is a uniquely allocated DMA frame and the request type
    // occupies the naturally aligned first u32 of the request header.
    unsafe {
        command_virtual
            .as_mut_ptr::<u32>()
            .write(VIRTIO_GPU_CMD_GET_DISPLAY_INFO)
    };

    let descriptors =
        (physical_memory_offset + info.descriptor_frame).as_mut_ptr::<VirtqDescriptor>();
    // SAFETY: queue-size validation reserves at least two complete descriptors
    // in the uniquely owned descriptor-table frame.
    unsafe {
        descriptors.write(VirtqDescriptor {
            address: command,
            length: GPU_CTRL_HEADER_SIZE,
            flags: VIRTQ_DESC_F_NEXT,
            next: 1,
        });
        descriptors.add(1).write(VirtqDescriptor {
            address: command + GPU_RESPONSE_OFFSET as u64,
            length: GPU_DISPLAY_INFO_SIZE,
            flags: VIRTQ_DESC_F_WRITE,
            next: 0,
        });
    }

    let available = physical_memory_offset + info.available_frame;
    // SAFETY: split-ring avail.idx is u16 at byte 2 and ring[0] is u16 at byte
    // 4 inside the dedicated driver-owned available-ring frame.
    unsafe {
        available.as_mut_ptr::<u16>().add(2).write(0);
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        available.as_mut_ptr::<u16>().add(1).write_volatile(1);
    }
    info.commands_submitted = 1;
    if !notify.write_u16(notify_offset as usize, CONTROL_QUEUE_INDEX) {
        return Err(TransportError::InvalidNotifyConfig);
    }

    let used = physical_memory_offset + info.used_frame;
    let mut completed = false;
    for _ in 0..COMMAND_POLL_LIMIT {
        // SAFETY: split-ring used.idx is device-written at byte 2 of the
        // dedicated used-ring frame and requires a volatile read.
        if unsafe { used.as_ptr::<u16>().add(1).read_volatile() } != 0 {
            completed = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !completed {
        return Err(TransportError::CommandTimeout);
    }
    core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
    info.commands_completed = 1;

    // SAFETY: used.idx completion returns the bounded response descriptor to
    // the driver; the response header starts at a four-byte aligned offset.
    let response = unsafe {
        command_virtual
            .as_ptr::<u8>()
            .add(GPU_RESPONSE_OFFSET)
            .cast::<u32>()
    };
    info.response_type = unsafe { response.read_volatile() };
    if info.response_type != VIRTIO_GPU_RESP_OK_DISPLAY_INFO {
        return Err(TransportError::InvalidGpuResponse);
    }

    for index in 0..16usize {
        let mode_offset = GPU_RESPONSE_OFFSET + 24 + index * 24;
        // SAFETY: all three fields are aligned and contained in the completed
        // 408-byte display-info response descriptor.
        let width = unsafe {
            command_virtual
                .as_ptr::<u8>()
                .add(mode_offset + 8)
                .cast::<u32>()
                .read_volatile()
        };
        let height = unsafe {
            command_virtual
                .as_ptr::<u8>()
                .add(mode_offset + 12)
                .cast::<u32>()
                .read_volatile()
        };
        let enabled = unsafe {
            command_virtual
                .as_ptr::<u8>()
                .add(mode_offset + 16)
                .cast::<u32>()
                .read_volatile()
        } != 0;
        if enabled {
            info.scanout_count = info.scanout_count.saturating_add(1);
            if info.scanout_count == 1 {
                info.primary_enabled = true;
                info.primary_width = width;
                info.primary_height = height;
            }
        }
    }
    Ok(())
}

fn initialize_primary_resource(
    pci_device: PciDevice,
    physical_memory_offset: VirtAddr,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    info: &mut VirtioGpuInfo,
) -> Result<(), TransportError> {
    let width = info.primary_width;
    let height = info.primary_height;
    if !info.primary_enabled || width == 0 || height == 0 {
        return Err(TransportError::InvalidDisplayMode);
    }
    let backing_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(TransportError::InvalidDisplayMode)?;
    let frame_count = backing_bytes.div_ceil(Size4KiB::SIZE);
    let backing_frame =
        allocate_contiguous_frames(physical_memory_offset, frame_allocator, frame_count)?;
    info.backing_frame = backing_frame;
    info.backing_bytes = backing_bytes;
    fill_test_pattern(physical_memory_offset, backing_frame, width, height);

    prepare_command(physical_memory_offset, info.command_frame, |request| {
        write_u32(request, 0, VIRTIO_GPU_CMD_RESOURCE_CREATE_2D);
        write_u32(request, 24, PRIMARY_RESOURCE_ID);
        write_u32(request, 28, VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM);
        write_u32(request, 32, width);
        write_u32(request, 36, height);
    });
    submit_control_command(
        pci_device,
        physical_memory_offset,
        info,
        40,
        GPU_CTRL_HEADER_SIZE,
        VIRTIO_GPU_RESP_OK_NODATA,
    )?;
    info.resource_created = true;

    prepare_command(physical_memory_offset, info.command_frame, |request| {
        write_u32(request, 0, VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING);
        write_u32(request, 24, PRIMARY_RESOURCE_ID);
        write_u32(request, 28, 1);
        write_u64(request, 32, backing_frame);
        write_u32(request, 40, backing_bytes as u32);
    });
    submit_control_command(
        pci_device,
        physical_memory_offset,
        info,
        48,
        GPU_CTRL_HEADER_SIZE,
        VIRTIO_GPU_RESP_OK_NODATA,
    )?;
    info.backing_attached = true;

    prepare_command(physical_memory_offset, info.command_frame, |request| {
        write_u32(request, 0, VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D);
        write_rect(request, width, height);
        write_u64(request, 40, 0);
        write_u32(request, 48, PRIMARY_RESOURCE_ID);
    });
    submit_control_command(
        pci_device,
        physical_memory_offset,
        info,
        56,
        GPU_CTRL_HEADER_SIZE,
        VIRTIO_GPU_RESP_OK_NODATA,
    )?;
    info.transfer_complete = true;

    prepare_command(physical_memory_offset, info.command_frame, |request| {
        write_u32(request, 0, VIRTIO_GPU_CMD_RESOURCE_FLUSH);
        write_rect(request, width, height);
        write_u32(request, 40, PRIMARY_RESOURCE_ID);
    });
    submit_control_command(
        pci_device,
        physical_memory_offset,
        info,
        48,
        GPU_CTRL_HEADER_SIZE,
        VIRTIO_GPU_RESP_OK_NODATA,
    )?;
    info.flush_complete = true;

    prepare_command(physical_memory_offset, info.command_frame, |request| {
        write_u32(request, 0, VIRTIO_GPU_CMD_SET_SCANOUT);
        write_rect(request, width, height);
        write_u32(request, 40, 0);
        write_u32(request, 44, PRIMARY_RESOURCE_ID);
    });
    submit_control_command(
        pci_device,
        physical_memory_offset,
        info,
        48,
        GPU_CTRL_HEADER_SIZE,
        VIRTIO_GPU_RESP_OK_NODATA,
    )?;
    info.scanout_configured = true;
    Ok(())
}

fn transfer_and_flush_region(
    pci_device: PciDevice,
    physical_memory_offset: VirtAddr,
    info: &mut VirtioGpuInfo,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<(), TransportError> {
    let backing_offset = (u64::from(y) * u64::from(info.primary_width) + u64::from(x)) * 4;
    prepare_command(physical_memory_offset, info.command_frame, |request| {
        write_u32(request, 0, VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D);
        write_rect_at(request, x, y, width, height);
        write_u64(request, 40, backing_offset);
        write_u32(request, 48, PRIMARY_RESOURCE_ID);
    });
    submit_control_command(
        pci_device,
        physical_memory_offset,
        info,
        56,
        GPU_CTRL_HEADER_SIZE,
        VIRTIO_GPU_RESP_OK_NODATA,
    )?;

    prepare_command(physical_memory_offset, info.command_frame, |request| {
        write_u32(request, 0, VIRTIO_GPU_CMD_RESOURCE_FLUSH);
        write_rect_at(request, x, y, width, height);
        write_u32(request, 40, PRIMARY_RESOURCE_ID);
    });
    submit_control_command(
        pci_device,
        physical_memory_offset,
        info,
        48,
        GPU_CTRL_HEADER_SIZE,
        VIRTIO_GPU_RESP_OK_NODATA,
    )
}

fn allocate_contiguous_frames(
    physical_memory_offset: VirtAddr,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    count: u64,
) -> Result<u64, TransportError> {
    if count == 0 || count > 4096 {
        return Err(TransportError::InvalidDisplayMode);
    }
    let first = allocate_zeroed_frame(physical_memory_offset, frame_allocator)?;
    for index in 1..count {
        let frame = allocate_zeroed_frame(physical_memory_offset, frame_allocator)?;
        if frame != first + index * Size4KiB::SIZE {
            return Err(TransportError::NonContiguousBacking);
        }
    }
    Ok(first)
}

fn fill_test_pattern(physical_memory_offset: VirtAddr, frame: u64, width: u32, height: u32) {
    let pixels = (physical_memory_offset + frame).as_mut_ptr::<u32>();
    for y in 0..height {
        for x in 0..width {
            let blue = x.saturating_mul(255) / width;
            let green = y.saturating_mul(255) / height;
            let red = if (x / 64 + y / 64) % 2 == 0 {
                0x40
            } else {
                0xa0
            };
            let value = blue | (green << 8) | (red << 16);
            // SAFETY: the backing allocation contains width*height u32 pixels.
            unsafe {
                pixels
                    .add((u64::from(y) * u64::from(width) + u64::from(x)) as usize)
                    .write(value)
            };
        }
    }
}

fn prepare_command(
    physical_memory_offset: VirtAddr,
    command_frame: u64,
    write: impl FnOnce(*mut u8),
) {
    let request = (physical_memory_offset + command_frame).as_mut_ptr::<u8>();
    // SAFETY: the command frame is exclusively owned by this serialized driver.
    unsafe { core::ptr::write_bytes(request, 0, Size4KiB::SIZE as usize) };
    write(request);
}

fn submit_control_command(
    pci_device: PciDevice,
    physical_memory_offset: VirtAddr,
    info: &mut VirtioGpuInfo,
    request_length: u32,
    response_length: u32,
    expected_response: u32,
) -> Result<(), TransportError> {
    let descriptors =
        (physical_memory_offset + info.descriptor_frame).as_mut_ptr::<VirtqDescriptor>();
    // SAFETY: descriptors 0 and 1 are within the validated control queue table.
    unsafe {
        descriptors.write(VirtqDescriptor {
            address: info.command_frame,
            length: request_length,
            flags: VIRTQ_DESC_F_NEXT,
            next: 1,
        });
        descriptors.add(1).write(VirtqDescriptor {
            address: info.command_frame + GPU_RESPONSE_OFFSET as u64,
            length: response_length,
            flags: VIRTQ_DESC_F_WRITE,
            next: 0,
        });
    }

    let target = (info.commands_submitted as u16).wrapping_add(1);
    let slot = info.commands_submitted as usize % usize::from(info.control_queue_size);
    let available = physical_memory_offset + info.available_frame;
    // SAFETY: slot is reduced modulo the negotiated queue size.
    unsafe {
        available.as_mut_ptr::<u16>().add(2 + slot).write(0);
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        available.as_mut_ptr::<u16>().add(1).write_volatile(target);
    }
    info.commands_submitted += 1;

    let common = capability_region(
        pci_device,
        info.common.ok_or(TransportError::MissingCommonConfig)?,
        physical_memory_offset,
    )?;
    common.write_u16(22, CONTROL_QUEUE_INDEX);
    let queue_notify_off = common
        .read_u16(30)
        .ok_or(TransportError::InvalidCommonConfig)?;
    let notify_offset = u64::from(queue_notify_off)
        .checked_mul(u64::from(info.notify_multiplier))
        .ok_or(TransportError::InvalidNotifyConfig)?;
    let notify_cap = info.notify.ok_or(TransportError::InvalidNotifyConfig)?;
    let notify = capability_region(pci_device, notify_cap, physical_memory_offset)?;
    if !notify.write_u16(notify_offset as usize, CONTROL_QUEUE_INDEX) {
        return Err(TransportError::InvalidNotifyConfig);
    }

    let used = physical_memory_offset + info.used_frame;
    for _ in 0..COMMAND_POLL_LIMIT {
        // SAFETY: used.idx is device-owned and lies inside the used-ring frame.
        if unsafe { used.as_ptr::<u16>().add(1).read_volatile() } == target {
            core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
            info.commands_completed += 1;
            let response =
                (physical_memory_offset + info.command_frame + GPU_RESPONSE_OFFSET as u64)
                    .as_ptr::<u32>();
            // SAFETY: the response descriptor has completed and contains its header.
            info.response_type = unsafe { response.read_volatile() };
            return if info.response_type == expected_response {
                Ok(())
            } else {
                Err(TransportError::InvalidGpuResponse)
            };
        }
        core::hint::spin_loop();
    }
    Err(TransportError::CommandTimeout)
}

fn write_u32(base: *mut u8, offset: usize, value: u32) {
    // SAFETY: callers provide the driver-owned, zeroed command page and all
    // command layouts use aligned u32 fields within that page.
    unsafe { base.add(offset).cast::<u32>().write(value) };
}

fn write_u64(base: *mut u8, offset: usize, value: u64) {
    // SAFETY: callers provide the driver-owned, zeroed command page and all
    // command layouts use aligned u64 fields within that page.
    unsafe { base.add(offset).cast::<u64>().write(value) };
}

fn write_rect(base: *mut u8, width: u32, height: u32) {
    write_rect_at(base, 0, 0, width, height);
}

fn write_rect_at(base: *mut u8, x: u32, y: u32, width: u32, height: u32) {
    write_u32(base, 24, x);
    write_u32(base, 28, y);
    write_u32(base, 32, width);
    write_u32(base, 36, height);
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
                    VIRTIO_PCI_CAP_NOTIFY_CFG => {
                        info.notify = Some(capability);
                        if cap_len >= 20 && pointer <= 0xe8 {
                            info.notify_multiplier = pci::read_config_u32(
                                device.bus,
                                device.slot,
                                device.function,
                                pointer + 16,
                            );
                        }
                    }
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
    writer.text(" commands=");
    writer.dec(info.commands_completed);
    writer.byte(b'/');
    writer.dec(info.commands_submitted);
    writer.text(" response=0x");
    writer.hex(u64::from(info.response_type), 4);
    writer.text(" scanouts=");
    writer.dec(u64::from(info.scanout_count));
    writer.text(" primary=");
    writer.dec(u64::from(info.primary_width));
    writer.byte(b'x');
    writer.dec(u64::from(info.primary_height));
    writer.text(" enabled=");
    writer.bool(info.primary_enabled);
    writer.text(" resource=");
    writer.bool(info.resource_created);
    writer.text(" backing=");
    writer.bool(info.backing_attached);
    writer.text(" transfer=");
    writer.bool(info.transfer_complete);
    writer.text(" flush=");
    writer.bool(info.flush_complete);
    writer.text(" scanout=");
    writer.bool(info.scanout_configured);
    writer.text(" bytes=");
    writer.dec(info.backing_bytes);
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
    fn hex(&mut self, value: u64, digits: usize) {
        for shift in (0..digits).rev() {
            let nibble = ((value >> (shift * 4)) & 0xf) as u8;
            self.byte(if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            });
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
