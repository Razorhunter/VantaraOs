use alloc::vec::Vec;
use core::sync::atomic::{Ordering, fence};
use lazy_static::lazy_static;

use crate::drivers::pci::{self, PciBar, PciDevice};
use crate::storage::block::{BLOCK_SIZE, BlockDevice, BlockError};
use crate::sync::PreemptMutex as Mutex;
use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{FrameAllocator, Mapper, Size4KiB},
};

const PCI_CLASS_MASS_STORAGE: u8 = 0x01;
const PCI_SUBCLASS_NVM: u8 = 0x08;
const PCI_PROG_IF_NVME: u8 = 0x02;
const NVME_BAR_INDEX: u8 = 0;
const NVME_MMIO_WINDOW: u64 = 0xffff_9100_0000_0000;
const NVME_MMIO_STRIDE: u64 = 0x4000;
const NVME_MMIO_SIZE: u64 = 0x4000;
const REG_CAP: u64 = 0x00;
const REG_VS: u64 = 0x08;
const REG_CC: u64 = 0x14;
const REG_CSTS: u64 = 0x1c;
const REG_AQA: u64 = 0x24;
const REG_ASQ: u64 = 0x28;
const REG_ACQ: u64 = 0x30;
const ADMIN_DOORBELL_BASE: u64 = 0x1000;
const CC_ENABLE: u32 = 1 << 0;
const CSTS_READY: u32 = 1 << 0;
const CSTS_FATAL: u32 = 1 << 1;
const ADMIN_QUEUE_MAX_DEPTH: u16 = 64;
const ADMIN_PAGE_SIZE: usize = 4096;
const CONTROLLER_POLL_LIMIT: usize = 2_000_000;
const ADMIN_IDENTIFY: u8 = 0x06;
const ADMIN_CREATE_IO_COMPLETION_QUEUE: u8 = 0x05;
const ADMIN_CREATE_IO_SUBMISSION_QUEUE: u8 = 0x01;
const ADMIN_SET_FEATURES: u8 = 0x09;
const NVM_FLUSH: u8 = 0x00;
const NVM_WRITE: u8 = 0x01;
const NVM_READ: u8 = 0x02;
const FEATURE_NUMBER_OF_QUEUES: u32 = 0x07;
const IDENTIFY_CONTROLLER: u32 = 1;
const IDENTIFY_NAMESPACE: u32 = 0;
const PRIMARY_NAMESPACE_ID: u32 = 1;
const PRIMARY_IO_QUEUE_ID: u16 = 1;
const IO_QUEUE_MAX_DEPTH: u16 = 64;
const READ_MARKER_LEN: usize = 8;
const SERIAL_LEN: usize = 20;
const MODEL_LEN: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NvmeController {
    pub pci: PciDevice,
    pub bar0: u64,
    pub prefetchable: bool,
    pub mmio_ready: bool,
    pub mmio_base: u64,
    physical_memory_offset: u64,
    pub capabilities: u64,
    pub version: u32,
    pub controller_config: u32,
    pub controller_status: u32,
    pub admin_queue_attributes: u32,
    pub admin_submission_queue: u64,
    pub admin_completion_queue: u64,
    pub max_queue_entries: u32,
    pub doorbell_stride: u32,
    pub min_page_size: u64,
    pub max_page_size: u64,
    pub owned: bool,
    pub admin_queue_ready: bool,
    pub admin_queue_depth: u16,
    pub identify_controller_ok: bool,
    pub identify_namespace_ok: bool,
    pub serial: [u8; SERIAL_LEN],
    pub serial_len: u8,
    pub model: [u8; MODEL_LEN],
    pub model_len: u8,
    pub namespace_count: u32,
    pub namespace_blocks: u64,
    pub namespace_capacity: u64,
    pub lba_size: u32,
    pub io_queue_ready: bool,
    pub io_queue_depth: u16,
    pub read_lba0_ok: bool,
    pub read_prefix: [u8; READ_MARKER_LEN],
    pub read_prefix_len: u8,
    admin_queue: Option<NvmeQueue>,
    io_queue: Option<NvmeQueue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NvmeQueue {
    submission: u64,
    completion: u64,
    data_buffer: u64,
    depth: u16,
    submission_tail: u16,
    completion_head: u16,
    completion_phase: bool,
    next_command_id: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NvmeSubmission {
    opcode: u8,
    flags: u8,
    command_id: u16,
    namespace_id: u32,
    reserved: u64,
    metadata: u64,
    data_pointer_1: u64,
    data_pointer_2: u64,
    command_specific: [u32; 6],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NvmeCompletion {
    result: u32,
    reserved: u32,
    submission_head: u16,
    submission_id: u16,
    command_id: u16,
    status_phase: u16,
}

const _: () = {
    assert!(core::mem::size_of::<NvmeSubmission>() == 64);
    assert!(core::mem::size_of::<NvmeCompletion>() == 16);
};

#[derive(Clone, Copy)]
struct NvmeMmio {
    base: u64,
}

impl NvmeMmio {
    fn new(base: u64) -> Self {
        Self { base }
    }

    fn read_u32(self, offset: u64) -> u32 {
        debug_assert!(offset < NVME_MMIO_SIZE && offset % 4 == 0);
        let pointer = (self.base + offset) as *const u32;
        // SAFETY: construction follows a successful mapping of the complete
        // NVMe MMIO window, and callers use aligned register offsets in range.
        unsafe { core::ptr::read_volatile(pointer) }
    }

    fn read_u64(self, offset: u64) -> u64 {
        debug_assert!(offset + 8 <= NVME_MMIO_SIZE && offset % 8 == 0);
        let pointer = (self.base + offset) as *const u64;
        // SAFETY: the mapped-range invariant matches `read_u32`; all 64-bit
        // accesses target naturally aligned NVMe controller registers.
        unsafe { core::ptr::read_volatile(pointer) }
    }

    fn write_u32(self, offset: u64, value: u32) {
        debug_assert!(offset < NVME_MMIO_SIZE && offset % 4 == 0);
        let pointer = (self.base + offset) as *mut u32;
        // SAFETY: the MMIO window and aligned-offset invariants match reads;
        // callers only mutate NVMe controller registers during ownership setup.
        unsafe { core::ptr::write_volatile(pointer, value) };
    }

    fn write_u64(self, offset: u64, value: u64) {
        debug_assert!(offset + 8 <= NVME_MMIO_SIZE && offset % 8 == 0);
        let pointer = (self.base + offset) as *mut u64;
        // SAFETY: the target is a naturally aligned writable 64-bit NVMe
        // register in the mapped controller window.
        unsafe { core::ptr::write_volatile(pointer, value) };
    }
}

lazy_static! {
    static ref CONTROLLERS: Mutex<Vec<NvmeController>> = Mutex::new(Vec::new());
}

pub fn init(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    physical_memory_offset: VirtAddr,
) {
    let candidates: Vec<NvmeController> = pci::devices_by_class(PCI_CLASS_MASS_STORAGE)
        .into_iter()
        .filter(|device| device.subclass == PCI_SUBCLASS_NVM && device.prog_if == PCI_PROG_IF_NVME)
        .filter_map(controller_from_pci)
        .collect();
    let controllers: Vec<NvmeController> = candidates
        .into_iter()
        .enumerate()
        .map(|(index, controller)| {
            map_controller(
                controller,
                index,
                mapper,
                frame_allocator,
                physical_memory_offset,
            )
        })
        .collect();

    if controllers.is_empty() {
        crate::drivers::status::report(
            "nvme",
            crate::drivers::status::DriverState::Missing,
            "no PCI NVMe controller",
        );
        crate::serial_println!("[NVME] no controller detected");
    } else {
        let queues_ready = controllers
            .iter()
            .any(|controller| controller.admin_queue_ready);
        let identify_ready = controllers
            .iter()
            .any(|controller| controller.identify_namespace_ok);
        let io_ready = controllers.iter().any(|controller| controller.read_lba0_ok);
        crate::drivers::status::report(
            "nvme",
            crate::drivers::status::DriverState::Degraded,
            if io_ready {
                "private I/O queues ready; single-block read complete"
            } else if identify_ready {
                "Identify complete; I/O queue setup failed"
            } else if queues_ready {
                "private admin queues ready; Identify failed"
            } else {
                "controller mapped; ownership transition failed"
            },
        );
        for controller in &controllers {
            crate::serial_println!(
                "[NVME] {:02x}:{:02x}.{} vendor={:04x} device={:04x} bar0={:#x} mmio_ready={} cap={:#018x} vs={:#010x} cc={:#010x} csts={:#010x} mqes={} dstrd={} mpsmin={} mpsmax={} owned={} adminq={} depth={} identify_ctrl={} identify_ns={} model={} serial={} namespaces={} nsid=1 blocks={} capacity={} lba_size={} ioq={} iodepth={} read_lba0={} data_prefix={} aqa={:#010x} asq={:#018x} acq={:#018x}",
                controller.pci.bus,
                controller.pci.slot,
                controller.pci.function,
                controller.pci.vendor_id,
                controller.pci.device_id,
                controller.bar0,
                controller.mmio_ready,
                controller.capabilities,
                controller.version,
                controller.controller_config,
                controller.controller_status,
                controller.max_queue_entries,
                controller.doorbell_stride,
                controller.min_page_size,
                controller.max_page_size,
                controller.owned,
                controller.admin_queue_ready,
                controller.admin_queue_depth,
                controller.identify_controller_ok,
                controller.identify_namespace_ok,
                controller.model_str(),
                controller.serial_str(),
                controller.namespace_count,
                controller.namespace_blocks,
                controller.namespace_capacity,
                controller.lba_size,
                controller.io_queue_ready,
                controller.io_queue_depth,
                controller.read_lba0_ok,
                controller.read_prefix_str(),
                controller.admin_queue_attributes,
                controller.admin_submission_queue,
                controller.admin_completion_queue
            );
        }
    }
    *CONTROLLERS.lock() = controllers;
    verify_block_device_path();
}

fn controller_from_pci(device: PciDevice) -> Option<NvmeController> {
    let (bar0, prefetchable) = match pci::read_bar_info(device, NVME_BAR_INDEX) {
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
    Some(NvmeController {
        pci: device,
        bar0,
        prefetchable,
        mmio_ready: false,
        mmio_base: 0,
        physical_memory_offset: 0,
        capabilities: 0,
        version: 0,
        controller_config: 0,
        controller_status: 0,
        admin_queue_attributes: 0,
        admin_submission_queue: 0,
        admin_completion_queue: 0,
        max_queue_entries: 0,
        doorbell_stride: 0,
        min_page_size: 0,
        max_page_size: 0,
        owned: false,
        admin_queue_ready: false,
        admin_queue_depth: 0,
        identify_controller_ok: false,
        identify_namespace_ok: false,
        serial: [0; SERIAL_LEN],
        serial_len: 0,
        model: [0; MODEL_LEN],
        model_len: 0,
        namespace_count: 0,
        namespace_blocks: 0,
        namespace_capacity: 0,
        lba_size: 0,
        io_queue_ready: false,
        io_queue_depth: 0,
        read_lba0_ok: false,
        read_prefix: [0; READ_MARKER_LEN],
        read_prefix_len: 0,
        admin_queue: None,
        io_queue: None,
    })
}

fn map_controller(
    mut controller: NvmeController,
    index: usize,
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    physical_memory_offset: VirtAddr,
) -> NvmeController {
    let Some(offset) = (index as u64).checked_mul(NVME_MMIO_STRIDE) else {
        return controller;
    };
    let Some(mmio_base) = NVME_MMIO_WINDOW.checked_add(offset) else {
        return controller;
    };
    if crate::memory::map_mmio_range(
        PhysAddr::new(controller.bar0),
        VirtAddr::new(mmio_base),
        NVME_MMIO_SIZE,
        mapper,
        frame_allocator,
    )
    .is_err()
    {
        return controller;
    }

    let registers = NvmeMmio::new(mmio_base);
    controller.mmio_base = mmio_base;
    controller.physical_memory_offset = physical_memory_offset.as_u64();
    controller.capabilities = registers.read_u64(REG_CAP);
    controller.version = registers.read_u32(REG_VS);
    controller.controller_config = registers.read_u32(REG_CC);
    controller.controller_status = registers.read_u32(REG_CSTS);
    controller.admin_queue_attributes = registers.read_u32(REG_AQA);
    controller.admin_submission_queue = registers.read_u64(REG_ASQ);
    controller.admin_completion_queue = registers.read_u64(REG_ACQ);
    controller.max_queue_entries = (controller.capabilities as u32 & 0xffff) + 1;
    controller.doorbell_stride = 4u32 << ((controller.capabilities >> 32) & 0x0f);
    controller.min_page_size = 1u64 << (12 + ((controller.capabilities >> 48) & 0x0f));
    controller.max_page_size = 1u64 << (12 + ((controller.capabilities >> 52) & 0x0f));
    controller.mmio_ready = true;
    controller.admin_queue = initialize_admin_queues(
        &mut controller,
        registers,
        frame_allocator,
        physical_memory_offset,
    );
    controller.admin_queue_ready = controller.admin_queue.is_some();
    controller.owned = controller.admin_queue_ready;
    if let Some(mut queue) = controller.admin_queue {
        if let Some(data) = submit_identify(
            registers,
            physical_memory_offset,
            controller.doorbell_stride,
            &mut queue,
            0,
            IDENTIFY_CONTROLLER,
        ) {
            parse_identify_controller(&mut controller, &data);
        }
        if controller.identify_controller_ok && controller.namespace_count != 0 {
            if let Some(data) = submit_identify(
                registers,
                physical_memory_offset,
                controller.doorbell_stride,
                &mut queue,
                PRIMARY_NAMESPACE_ID,
                IDENTIFY_NAMESPACE,
            ) {
                parse_identify_namespace(&mut controller, &data);
            }
        }
        if controller.identify_namespace_ok && controller.lba_size as usize <= ADMIN_PAGE_SIZE {
            if let Some(mut io_queue) = initialize_io_queue(
                registers,
                physical_memory_offset,
                controller.doorbell_stride,
                controller.max_queue_entries,
                frame_allocator,
                &mut queue,
            ) {
                controller.io_queue_ready = true;
                controller.io_queue_depth = io_queue.depth;
                if let Some(data) = submit_nvm_read(
                    registers,
                    physical_memory_offset,
                    controller.doorbell_stride,
                    &mut io_queue,
                    PRIMARY_NAMESPACE_ID,
                    0,
                ) {
                    let (prefix, prefix_len) = parse_ascii::<READ_MARKER_LEN>(&data, 0);
                    controller.read_prefix = prefix;
                    controller.read_prefix_len = prefix_len as u8;
                    controller.read_lba0_ok = true;
                }
                controller.io_queue = Some(io_queue);
            }
        }
        controller.admin_queue = Some(queue);
    }
    controller.controller_config = registers.read_u32(REG_CC);
    controller.controller_status = registers.read_u32(REG_CSTS);
    controller.admin_queue_attributes = registers.read_u32(REG_AQA);
    controller.admin_submission_queue = registers.read_u64(REG_ASQ);
    controller.admin_completion_queue = registers.read_u64(REG_ACQ);
    controller
}

fn initialize_admin_queues(
    controller: &mut NvmeController,
    registers: NvmeMmio,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    physical_memory_offset: VirtAddr,
) -> Option<NvmeQueue> {
    if controller.min_page_size > ADMIN_PAGE_SIZE as u64
        || controller.max_page_size < ADMIN_PAGE_SIZE as u64
        || controller.max_queue_entries < 2
    {
        return None;
    }
    let Some(submission_frame) = frame_allocator.allocate_frame() else {
        return None;
    };
    let Some(completion_frame) = frame_allocator.allocate_frame() else {
        return None;
    };
    let Some(data_frame) = frame_allocator.allocate_frame() else {
        return None;
    };
    let submission = submission_frame.start_address().as_u64();
    let completion = completion_frame.start_address().as_u64();
    let data_buffer = data_frame.start_address().as_u64();
    zero_nvme_pages(physical_memory_offset, submission, completion, data_buffer);

    let disabled = registers.read_u32(REG_CC) & !CC_ENABLE;
    registers.write_u32(REG_CC, disabled);
    fence(Ordering::SeqCst);
    if !wait_controller_ready(registers, false) {
        return None;
    }

    let depth = controller
        .max_queue_entries
        .min(u32::from(ADMIN_QUEUE_MAX_DEPTH)) as u16;
    let zero_based = u32::from(depth - 1);
    registers.write_u32(REG_AQA, (zero_based << 16) | zero_based);
    registers.write_u64(REG_ASQ, submission);
    registers.write_u64(REG_ACQ, completion);
    let config = controller_config_4k();
    fence(Ordering::SeqCst);
    registers.write_u32(REG_CC, config | CC_ENABLE);
    fence(Ordering::SeqCst);
    if !wait_controller_ready(registers, true) {
        return None;
    }
    controller.admin_queue_depth = depth;
    Some(NvmeQueue {
        submission,
        completion,
        data_buffer,
        depth,
        submission_tail: 0,
        completion_head: 0,
        completion_phase: true,
        next_command_id: 1,
    })
}

fn controller_config_4k() -> u32 {
    const IOSQES_64_BYTES: u32 = 6 << 16;
    const IOCQES_16_BYTES: u32 = 4 << 20;
    IOSQES_64_BYTES | IOCQES_16_BYTES
}

fn initialize_io_queue(
    registers: NvmeMmio,
    physical_memory_offset: VirtAddr,
    doorbell_stride: u32,
    max_queue_entries: u32,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    admin_queue: &mut NvmeQueue,
) -> Option<NvmeQueue> {
    let Some(submission_frame) = frame_allocator.allocate_frame() else {
        return None;
    };
    let Some(completion_frame) = frame_allocator.allocate_frame() else {
        return None;
    };
    let Some(data_frame) = frame_allocator.allocate_frame() else {
        return None;
    };
    let submission = submission_frame.start_address().as_u64();
    let completion = completion_frame.start_address().as_u64();
    let data_buffer = data_frame.start_address().as_u64();
    zero_nvme_pages(physical_memory_offset, submission, completion, data_buffer);
    let depth = max_queue_entries.min(u32::from(IO_QUEUE_MAX_DEPTH)) as u16;
    if depth < 2 {
        return None;
    }
    let zero_based = u32::from(depth - 1);
    let mut no_output: [u8; 0] = [];

    let mut set_queues = empty_submission(ADMIN_SET_FEATURES, 0, 0);
    set_queues.command_specific[0] = FEATURE_NUMBER_OF_QUEUES;
    set_queues.command_specific[1] = 0;
    submit_queue_command(
        registers,
        physical_memory_offset,
        doorbell_stride,
        admin_queue,
        0,
        set_queues,
        false,
        None,
        &mut no_output,
    )
    .then_some(())?;

    let mut create_completion = empty_submission(ADMIN_CREATE_IO_COMPLETION_QUEUE, 0, completion);
    create_completion.command_specific[0] = u32::from(PRIMARY_IO_QUEUE_ID) | (zero_based << 16);
    create_completion.command_specific[1] = 1;
    submit_queue_command(
        registers,
        physical_memory_offset,
        doorbell_stride,
        admin_queue,
        0,
        create_completion,
        false,
        None,
        &mut no_output,
    )
    .then_some(())?;

    let mut create_submission = empty_submission(ADMIN_CREATE_IO_SUBMISSION_QUEUE, 0, submission);
    create_submission.command_specific[0] = u32::from(PRIMARY_IO_QUEUE_ID) | (zero_based << 16);
    create_submission.command_specific[1] = 1 | (u32::from(PRIMARY_IO_QUEUE_ID) << 16);
    submit_queue_command(
        registers,
        physical_memory_offset,
        doorbell_stride,
        admin_queue,
        0,
        create_submission,
        false,
        None,
        &mut no_output,
    )
    .then_some(())?;

    Some(NvmeQueue {
        submission,
        completion,
        data_buffer,
        depth,
        submission_tail: 0,
        completion_head: 0,
        completion_phase: true,
        next_command_id: 1,
    })
}

fn submit_nvm_read(
    registers: NvmeMmio,
    physical_memory_offset: VirtAddr,
    doorbell_stride: u32,
    queue: &mut NvmeQueue,
    namespace_id: u32,
    lba: u64,
) -> Option<[u8; BLOCK_SIZE]> {
    let mut command = empty_submission(NVM_READ, namespace_id, queue.data_buffer);
    command.command_specific[0] = lba as u32;
    command.command_specific[1] = (lba >> 32) as u32;
    command.command_specific[2] = 0;
    let mut output = [0u8; BLOCK_SIZE];
    submit_queue_command(
        registers,
        physical_memory_offset,
        doorbell_stride,
        queue,
        PRIMARY_IO_QUEUE_ID,
        command,
        true,
        None,
        &mut output,
    )
    .then_some(output)
}

fn submit_nvm_write(
    registers: NvmeMmio,
    physical_memory_offset: VirtAddr,
    doorbell_stride: u32,
    queue: &mut NvmeQueue,
    namespace_id: u32,
    lba: u64,
    data: &[u8; 512],
) -> bool {
    let mut command = empty_submission(NVM_WRITE, namespace_id, queue.data_buffer);
    command.command_specific[0] = lba as u32;
    command.command_specific[1] = (lba >> 32) as u32;
    command.command_specific[2] = 0;
    let mut output = [];
    submit_queue_command(
        registers,
        physical_memory_offset,
        doorbell_stride,
        queue,
        PRIMARY_IO_QUEUE_ID,
        command,
        false,
        Some(data),
        &mut output,
    )
}

fn submit_nvm_flush(
    registers: NvmeMmio,
    physical_memory_offset: VirtAddr,
    doorbell_stride: u32,
    queue: &mut NvmeQueue,
    namespace_id: u32,
) -> bool {
    let command = empty_submission(NVM_FLUSH, namespace_id, 0);
    let mut output = [];
    submit_queue_command(
        registers,
        physical_memory_offset,
        doorbell_stride,
        queue,
        PRIMARY_IO_QUEUE_ID,
        command,
        false,
        None,
        &mut output,
    )
}

fn empty_submission(opcode: u8, namespace_id: u32, data_pointer_1: u64) -> NvmeSubmission {
    NvmeSubmission {
        opcode,
        flags: 0,
        command_id: 0,
        namespace_id,
        reserved: 0,
        metadata: 0,
        data_pointer_1,
        data_pointer_2: 0,
        command_specific: [0; 6],
    }
}

fn wait_controller_ready(registers: NvmeMmio, expected: bool) -> bool {
    for _ in 0..CONTROLLER_POLL_LIMIT {
        let status = registers.read_u32(REG_CSTS);
        if status & CSTS_FATAL != 0 {
            return false;
        }
        if (status & CSTS_READY != 0) == expected {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn zero_nvme_pages(
    physical_memory_offset: VirtAddr,
    submission: u64,
    completion: u64,
    data_buffer: u64,
) {
    let submission_pointer = (physical_memory_offset + submission).as_mut_ptr::<u8>();
    let completion_pointer = (physical_memory_offset + completion).as_mut_ptr::<u8>();
    let data_pointer = (physical_memory_offset + data_buffer).as_mut_ptr::<u8>();
    // SAFETY: all three physical frames were just allocated exclusively for
    // NVMe queues/data and the direct mapping covers each complete page.
    unsafe {
        core::ptr::write_bytes(submission_pointer, 0, ADMIN_PAGE_SIZE);
        core::ptr::write_bytes(completion_pointer, 0, ADMIN_PAGE_SIZE);
        core::ptr::write_bytes(data_pointer, 0, ADMIN_PAGE_SIZE);
    }
}

fn submit_identify(
    registers: NvmeMmio,
    physical_memory_offset: VirtAddr,
    doorbell_stride: u32,
    queue: &mut NvmeQueue,
    namespace_id: u32,
    controller_or_namespace: u32,
) -> Option<[u8; ADMIN_PAGE_SIZE]> {
    let command = NvmeSubmission {
        opcode: ADMIN_IDENTIFY,
        flags: 0,
        command_id: 0,
        namespace_id,
        reserved: 0,
        metadata: 0,
        data_pointer_1: queue.data_buffer,
        data_pointer_2: 0,
        command_specific: [controller_or_namespace, 0, 0, 0, 0, 0],
    };
    let mut output = [0u8; ADMIN_PAGE_SIZE];
    submit_queue_command(
        registers,
        physical_memory_offset,
        doorbell_stride,
        queue,
        0,
        command,
        true,
        None,
        &mut output,
    )
    .then_some(output)
}

fn submit_queue_command(
    registers: NvmeMmio,
    physical_memory_offset: VirtAddr,
    doorbell_stride: u32,
    queue: &mut NvmeQueue,
    queue_id: u16,
    mut command: NvmeSubmission,
    clear_data: bool,
    data_input: Option<&[u8]>,
    output: &mut [u8],
) -> bool {
    let command_id = queue.next_command_id;
    queue.next_command_id = queue.next_command_id.wrapping_add(1);
    command.command_id = command_id;
    let submission_pointer =
        (physical_memory_offset + queue.submission).as_mut_ptr::<NvmeSubmission>();
    let completion_pointer = (physical_memory_offset + queue.completion).as_ptr::<NvmeCompletion>();
    let data_pointer = (physical_memory_offset + queue.data_buffer).as_mut_ptr::<u8>();

    // SAFETY: queue and data frames are exclusive, page-aligned DMA memory;
    // indices are bounded by the configured depth. Volatile queue accesses and
    // fences synchronize with the controller before the completed data copy.
    unsafe {
        if clear_data {
            core::ptr::write_bytes(data_pointer, 0, ADMIN_PAGE_SIZE);
        }
        if let Some(data) = data_input {
            if data.len() > ADMIN_PAGE_SIZE {
                return false;
            }
            core::ptr::copy_nonoverlapping(data.as_ptr(), data_pointer, data.len());
        }
        core::ptr::write_volatile(
            submission_pointer.add(queue.submission_tail as usize),
            command,
        );
        queue.submission_tail = (queue.submission_tail + 1) % queue.depth;
        fence(Ordering::SeqCst);
        let submission_doorbell =
            ADMIN_DOORBELL_BASE + u64::from(2 * u32::from(queue_id) * doorbell_stride);
        registers.write_u32(submission_doorbell, u32::from(queue.submission_tail));

        let mut completed = None;
        for _ in 0..CONTROLLER_POLL_LIMIT {
            let entry =
                core::ptr::read_volatile(completion_pointer.add(queue.completion_head as usize));
            if (entry.status_phase & 1 != 0) == queue.completion_phase {
                completed = Some(entry);
                break;
            }
            core::hint::spin_loop();
        }
        let Some(completion) = completed else {
            crate::serial_println!(
                "[NVME-IO] timeout qid={} cid={} sq_tail={} cq_head={} phase={}",
                queue_id,
                command_id,
                queue.submission_tail,
                queue.completion_head,
                queue.completion_phase
            );
            return false;
        };
        fence(Ordering::SeqCst);
        if completion.command_id != command_id
            || completion.submission_id != queue_id
            || completion.status_phase >> 1 != 0
        {
            crate::serial_println!(
                "[NVME-IO] completion-error qid={} cid={} got_qid={} got_cid={} status={:#06x}",
                queue_id,
                command_id,
                completion.submission_id,
                completion.command_id,
                completion.status_phase >> 1
            );
            return false;
        }

        queue.completion_head += 1;
        if queue.completion_head == queue.depth {
            queue.completion_head = 0;
            queue.completion_phase = !queue.completion_phase;
        }
        let completion_doorbell =
            ADMIN_DOORBELL_BASE + u64::from((2 * u32::from(queue_id) + 1) * doorbell_stride);
        registers.write_u32(completion_doorbell, u32::from(queue.completion_head));
        fence(Ordering::SeqCst);
        core::ptr::copy_nonoverlapping(data_pointer, output.as_mut_ptr(), output.len());
    }
    true
}

#[cfg(feature = "nvme-write-test")]
pub fn run_write_test() {
    const MAGIC: &[u8] = b"VANTARA_NVME_WRITE_V1";
    let result = (|| {
        let mut controllers = CONTROLLERS.lock();
        let controller = controllers
            .iter_mut()
            .find(|controller| controller.io_queue_ready)?;
        if controller.lba_size != 512 || controller.namespace_blocks == 0 {
            return None;
        }
        let lba = controller.namespace_blocks - 1;
        let registers = NvmeMmio::new(controller.mmio_base);
        let physical_memory_offset = VirtAddr::new(controller.physical_memory_offset);
        let doorbell_stride = controller.doorbell_stride;
        let queue = controller.io_queue.as_mut()?;
        let existing = submit_nvm_read(
            registers,
            physical_memory_offset,
            doorbell_stride,
            queue,
            PRIMARY_NAMESPACE_ID,
            lba,
        )?;
        let mut marker = [0u8; 512];
        marker[..MAGIC.len()].copy_from_slice(MAGIC);
        for (index, byte) in marker[MAGIC.len()..].iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(29).wrapping_add(0x37);
        }
        let phase = if existing[..512] == marker {
            "verify"
        } else {
            if !submit_nvm_write(
                registers,
                physical_memory_offset,
                doorbell_stride,
                queue,
                PRIMARY_NAMESPACE_ID,
                lba,
                &marker,
            ) || !submit_nvm_flush(
                registers,
                physical_memory_offset,
                doorbell_stride,
                queue,
                PRIMARY_NAMESPACE_ID,
            ) {
                return None;
            }
            let readback = submit_nvm_read(
                registers,
                physical_memory_offset,
                doorbell_stride,
                queue,
                PRIMARY_NAMESPACE_ID,
                lba,
            )?;
            if readback[..512] != marker {
                return None;
            }
            "write"
        };
        let checksum = marker
            .iter()
            .fold(0u64, |sum, byte| sum.wrapping_add(u64::from(*byte)));
        Some((phase, lba, checksum))
    })();
    match result {
        Some((phase, lba, checksum)) => {
            crate::serial_println!(
                "[NVME-WRITE-TEST] phase={} lba={} checksum={} persisted=true flush=true",
                phase,
                lba,
                checksum
            );
        }
        None => {
            crate::serial_println!("[NVME-WRITE-TEST] failed");
        }
    }
}

fn parse_identify_controller(controller: &mut NvmeController, data: &[u8; ADMIN_PAGE_SIZE]) {
    let (serial, serial_len) = parse_ascii::<SERIAL_LEN>(data, 4);
    let (model, model_len) = parse_ascii::<MODEL_LEN>(data, 24);
    controller.serial = serial;
    controller.serial_len = serial_len as u8;
    controller.model = model;
    controller.model_len = model_len as u8;
    controller.namespace_count = read_u32(data, 516);
    controller.identify_controller_ok = true;
}

fn parse_identify_namespace(controller: &mut NvmeController, data: &[u8; ADMIN_PAGE_SIZE]) {
    let blocks = read_u64(data, 0);
    let capacity = read_u64(data, 8);
    let format = usize::from(data[26] & 0x0f);
    let format_offset = 128 + format * 4;
    let lba_shift = data.get(format_offset + 2).copied().unwrap_or(0);
    let lba_size = 1u32.checked_shl(u32::from(lba_shift)).unwrap_or(0);
    if blocks == 0 || capacity == 0 || lba_size == 0 {
        return;
    }
    controller.namespace_blocks = blocks;
    controller.namespace_capacity = capacity;
    controller.lba_size = lba_size;
    controller.identify_namespace_ok = true;
}

fn parse_ascii<const N: usize>(data: &[u8], offset: usize) -> ([u8; N], usize) {
    let mut value = [b' '; N];
    for (index, byte) in value.iter_mut().enumerate() {
        let source = data.get(offset + index).copied().unwrap_or(0);
        *byte = if (b' '..=b'~').contains(&source) {
            source
        } else if source == 0 {
            b' '
        } else {
            b'?'
        };
    }
    let len = value
        .iter()
        .rposition(|byte| *byte != b' ')
        .map_or(0, |index| index + 1);
    (value, len)
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap_or([0; 4]))
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap_or([0; 8]))
}

impl NvmeController {
    fn model_str(&self) -> &str {
        core::str::from_utf8(&self.model[..self.model_len as usize]).unwrap_or("?")
    }

    fn serial_str(&self) -> &str {
        core::str::from_utf8(&self.serial[..self.serial_len as usize]).unwrap_or("?")
    }

    fn read_prefix_str(&self) -> &str {
        core::str::from_utf8(&self.read_prefix[..self.read_prefix_len as usize]).unwrap_or("?")
    }
}

pub struct NvmeBlockDevice {
    controller_index: usize,
    blocks: u64,
}

impl NvmeBlockDevice {
    pub fn primary() -> Result<Self, BlockError> {
        let controllers = CONTROLLERS.lock();
        for (controller_index, controller) in controllers.iter().enumerate() {
            if controller.io_queue_ready
                && controller.lba_size as usize == BLOCK_SIZE
                && controller.namespace_blocks != 0
            {
                return Ok(Self {
                    controller_index,
                    blocks: controller.namespace_blocks,
                });
            }
        }
        Err(BlockError::NoDevice)
    }
}

impl BlockDevice for NvmeBlockDevice {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(
        &mut self,
        block_index: u64,
        buffer: &mut [u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        if block_index >= self.blocks {
            crate::serial_println!(
                "[NVME-IO] read out-of-range lba={} blocks={}",
                block_index,
                self.blocks
            );
            return Err(BlockError::OutOfRange);
        }
        let mut controllers = CONTROLLERS.lock();
        let Some(controller) = controllers.get_mut(self.controller_index) else {
            crate::serial_println!(
                "[NVME-IO] missing controller index={} count={}",
                self.controller_index,
                controllers.len()
            );
            return Err(BlockError::NoDevice);
        };
        let registers = NvmeMmio::new(controller.mmio_base);
        let physical_memory_offset = VirtAddr::new(controller.physical_memory_offset);
        let doorbell_stride = controller.doorbell_stride;
        let Some(queue) = controller.io_queue.as_mut() else {
            crate::serial_println!(
                "[NVME-IO] missing I/O queue controller={}",
                self.controller_index
            );
            return Err(BlockError::NoDevice);
        };
        let data = submit_nvm_read(
            registers,
            physical_memory_offset,
            doorbell_stride,
            queue,
            PRIMARY_NAMESPACE_ID,
            block_index,
        )
        .ok_or(BlockError::DeviceFault)?;
        buffer.copy_from_slice(&data[..BLOCK_SIZE]);
        Ok(())
    }

    fn write_block(
        &mut self,
        block_index: u64,
        buffer: &[u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        if block_index >= self.blocks {
            return Err(BlockError::OutOfRange);
        }
        let mut controllers = CONTROLLERS.lock();
        let controller = controllers
            .get_mut(self.controller_index)
            .ok_or(BlockError::NoDevice)?;
        let registers = NvmeMmio::new(controller.mmio_base);
        let physical_memory_offset = VirtAddr::new(controller.physical_memory_offset);
        let doorbell_stride = controller.doorbell_stride;
        let queue = controller.io_queue.as_mut().ok_or(BlockError::NoDevice)?;
        if !submit_nvm_write(
            registers,
            physical_memory_offset,
            doorbell_stride,
            queue,
            PRIMARY_NAMESPACE_ID,
            block_index,
            buffer,
        ) || !submit_nvm_flush(
            registers,
            physical_memory_offset,
            doorbell_stride,
            queue,
            PRIMARY_NAMESPACE_ID,
        ) {
            return Err(BlockError::DeviceFault);
        }
        Ok(())
    }
}

fn verify_block_device_path() {
    let result = (|| {
        let mut disk = NvmeBlockDevice::primary()?;
        let blocks = disk.block_count();
        let mut sector = [0u8; BLOCK_SIZE];
        disk.read_block(0, &mut sector)?;
        if !sector.starts_with(b"VANTNVME") {
            return Err(BlockError::DeviceFault);
        }
        Ok(blocks)
    })();
    match result {
        Ok(blocks) => {
            crate::drivers::status::report(
                "nvme-block",
                crate::drivers::status::DriverState::Ready,
                "synchronized NVM BlockDevice with flush",
            );
            crate::serial_println!(
                "[NVME-BLOCK] ready blocks={} read_lba0=true writable=true flush=true",
                blocks
            );
        }
        Err(error) => {
            crate::drivers::status::report(
                "nvme-block",
                crate::drivers::status::DriverState::Missing,
                "NVM BlockDevice unavailable",
            );
            crate::serial_println!("[NVME-BLOCK] unavailable error={:?}", error);
        }
    }
}

pub fn write_to_buffer(out: &mut [u8]) -> usize {
    let controllers = CONTROLLERS.lock();
    let mut writer = BufferWriter::new(out);
    writer.write_str("NVMe controllers: ");
    writer.write_dec(controllers.len() as u64);
    writer.write_byte(b'\n');
    writer.write_str("BDF       VENDOR:DEVICE BAR0               CAP              VS       CC       CSTS     MQES DSTRD MPSMIN MPSMAX OWN ADMINQ DEPTH IDENTIFY MODEL SERIAL NS BLOCKS LBA IOQ IODEPTH READ PREFIX STATE\n");
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
        writer.write_hex_u64(controller.bar0);
        writer.write_byte(b' ');
        writer.write_hex_u64(controller.capabilities);
        writer.write_byte(b' ');
        writer.write_hex_u32(controller.version);
        writer.write_byte(b' ');
        writer.write_hex_u32(controller.controller_config);
        writer.write_byte(b' ');
        writer.write_hex_u32(controller.controller_status);
        writer.write_byte(b' ');
        writer.write_dec(controller.max_queue_entries as u64);
        writer.write_byte(b' ');
        writer.write_dec(controller.doorbell_stride as u64);
        writer.write_byte(b' ');
        writer.write_dec(controller.min_page_size);
        writer.write_byte(b' ');
        writer.write_dec(controller.max_page_size);
        writer.write_byte(b' ');
        writer.write_dec(controller.owned as u64);
        writer.write_byte(b' ');
        writer.write_dec(controller.admin_queue_ready as u64);
        writer.write_byte(b' ');
        writer.write_dec(controller.admin_queue_depth as u64);
        writer.write_byte(b' ');
        writer.write_dec(controller.identify_namespace_ok as u64);
        writer.write_byte(b' ');
        writer.write_str(controller.model_str());
        writer.write_byte(b' ');
        writer.write_str(controller.serial_str());
        writer.write_byte(b' ');
        writer.write_dec(controller.namespace_count as u64);
        writer.write_byte(b' ');
        writer.write_dec(controller.namespace_blocks);
        writer.write_byte(b' ');
        writer.write_dec(controller.lba_size as u64);
        writer.write_byte(b' ');
        writer.write_dec(controller.io_queue_ready as u64);
        writer.write_byte(b' ');
        writer.write_dec(controller.io_queue_depth as u64);
        writer.write_byte(b' ');
        writer.write_dec(controller.read_lba0_ok as u64);
        writer.write_byte(b' ');
        writer.write_str(controller.read_prefix_str());
        writer.write_str(if controller.mmio_ready {
            " mapped\n"
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

    fn write_hex_u32(&mut self, value: u32) {
        for shift in (0..8).rev() {
            self.write_hex_nibble((value >> (shift * 4)) as u8);
        }
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
    use super::{ADMIN_PAGE_SIZE, BufferWriter, controller_config_4k, parse_ascii};

    #[test_case]
    fn formats_full_width_nvme_bar() {
        let mut out = [0u8; 16];
        let mut writer = BufferWriter::new(&mut out);
        writer.write_hex_u64(0x1234_abcd);
        let len = writer.len();
        drop(writer);
        assert_eq!(&out[..len], b"000000001234abcd");
    }

    #[test_case]
    fn configures_standard_admin_queue_entry_sizes() {
        let config = controller_config_4k();
        assert_eq!((config >> 16) & 0x0f, 6);
        assert_eq!((config >> 20) & 0x0f, 4);
        assert_eq!((config >> 7) & 0x0f, 0);
    }

    #[test_case]
    fn trims_identify_ascii_fields() {
        let mut data = [0u8; ADMIN_PAGE_SIZE];
        data[24..34].copy_from_slice(b"Vantara   ");
        let (value, len) = parse_ascii::<10>(&data, 24);
        assert_eq!(&value[..len], b"Vantara");
    }
}
