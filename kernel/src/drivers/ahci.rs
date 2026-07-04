use alloc::vec::Vec;
use core::sync::atomic::{Ordering, fence};
use lazy_static::lazy_static;

use crate::drivers::pci::{self, PciBar, PciDevice};
use crate::storage::block::{BLOCK_SIZE, BlockDevice, BlockError};
use crate::sync::PreemptMutex as Mutex;
use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{FrameAllocator, Mapper, PageSize, Size4KiB},
};

const PCI_CLASS_MASS_STORAGE: u8 = 0x01;
const PCI_SUBCLASS_SATA: u8 = 0x06;
const PCI_PROG_IF_AHCI: u8 = 0x01;
const AHCI_ABAR_INDEX: u8 = 5;
const AHCI_MMIO_WINDOW: u64 = 0xffff_9000_0000_0000;
const AHCI_MMIO_STRIDE: u64 = 0x2000;
const AHCI_MMIO_SIZE: u64 = 0x2000;
const REG_CAP: u64 = 0x00;
const REG_GHC: u64 = 0x04;
const REG_PORTS_IMPLEMENTED: u64 = 0x0c;
const REG_VERSION: u64 = 0x10;
const PORT_BASE: u64 = 0x100;
const PORT_STRIDE: u64 = 0x80;
const PORT_CLB: u64 = 0x00;
const PORT_CLBU: u64 = 0x04;
const PORT_FB: u64 = 0x08;
const PORT_FBU: u64 = 0x0c;
const PORT_IS: u64 = 0x10;
const PORT_IE: u64 = 0x14;
const PORT_CMD: u64 = 0x18;
const PORT_TFD: u64 = 0x20;
const PORT_SIG: u64 = 0x24;
const PORT_SSTS: u64 = 0x28;
const PORT_SERR: u64 = 0x30;
const PORT_SACT: u64 = 0x34;
const PORT_CI: u64 = 0x38;
const PORT_CMD_ST: u32 = 1 << 0;
const PORT_CMD_FRE: u32 = 1 << 4;
const PORT_CMD_FR: u32 = 1 << 14;
const PORT_CMD_CR: u32 = 1 << 15;
const PORT_IS_TFES: u32 = 1 << 30;
const PORT_TFD_DRQ: u32 = 1 << 3;
const PORT_TFD_BSY: u32 = 1 << 7;
const SATA_SIGNATURE: u32 = 0x0000_0101;
const PORT_POLL_LIMIT: usize = 1_000_000;
const RECEIVED_FIS_OFFSET: u64 = 1024;
const PRDT_OFFSET: usize = 0x80;
const COMMAND_FIS_DWORDS: u16 = 5;
const COMMAND_HEADER_WRITE: u16 = 1 << 6;
const PRDT_BYTE_COUNT: u32 = 512;
const FIS_TYPE_REG_H2D: u8 = 0x27;
const FIS_COMMAND: u8 = 1 << 7;
const ATA_IDENTIFY_DEVICE: u8 = 0xec;
const ATA_READ_DMA_EXT: u8 = 0x25;
const ATA_WRITE_DMA_EXT: u8 = 0x35;
const ATA_FLUSH_CACHE_EXT: u8 = 0xea;
const ATA_DEVICE_LBA: u8 = 1 << 6;
const ATA_MODEL_OFFSET: usize = 27 * 2;
const ATA_MODEL_LEN: usize = 40;
const BOOT_SIGNATURE_OFFSET: usize = 510;
const BOOT_SIGNATURE: u16 = 0x55aa;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AhciController {
    pub pci: PciDevice,
    pub abar: u64,
    pub prefetchable: bool,
    pub mmio_ready: bool,
    pub mmio_base: u64,
    pub capabilities: u32,
    pub global_host_control: u32,
    pub ports_implemented: u32,
    pub version: u32,
    pub physical_memory_offset: u64,
    pub ports: [Option<AhciPort>; 32],
    pub rebased_port_count: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AhciPort {
    pub index: u8,
    pub signature: u32,
    pub sata_status: u32,
    pub active: bool,
    pub rebased: bool,
    pub command_list: u64,
    pub received_fis: u64,
    pub command_table: u64,
    pub data_buffer: u64,
    pub identify_ok: bool,
    pub model: [u8; ATA_MODEL_LEN],
    pub model_len: u8,
    pub block_count: u64,
    pub read_lba0_ok: bool,
    pub lba0_signature: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AhciCommandHeader {
    flags: u16,
    prdt_len: u16,
    transferred: u32,
    table_base: u32,
    table_base_upper: u32,
    reserved: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AhciPrdtEntry {
    data_base: u32,
    data_base_upper: u32,
    reserved: u32,
    byte_count_interrupt: u32,
}

struct IdentifyData {
    model: [u8; ATA_MODEL_LEN],
    model_len: usize,
    block_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AhciBlockDevice {
    controller_index: usize,
    port_index: u8,
    blocks: u64,
}

const _: () = {
    assert!(core::mem::size_of::<AhciCommandHeader>() == 32);
    assert!(core::mem::size_of::<AhciPrdtEntry>() == 16);
};

#[derive(Clone, Copy)]
struct AhciMmio {
    base: u64,
}

impl AhciMmio {
    fn new(base: u64) -> Self {
        Self { base }
    }

    fn global_read(self, offset: u64) -> u32 {
        debug_assert!(offset < PORT_BASE && offset % 4 == 0);
        self.read_at(offset)
    }

    fn port_read(self, port: u8, offset: u64) -> u32 {
        debug_assert!(port < 32 && offset < PORT_STRIDE && offset % 4 == 0);
        self.read_at(PORT_BASE + u64::from(port) * PORT_STRIDE + offset)
    }

    fn port_write(self, port: u8, offset: u64, value: u32) {
        debug_assert!(port < 32 && offset < PORT_STRIDE && offset % 4 == 0);
        self.write_at(PORT_BASE + u64::from(port) * PORT_STRIDE + offset, value);
    }

    fn read_at(self, offset: u64) -> u32 {
        let pointer = (self.base + offset) as *const u32;
        // SAFETY: construction follows a successful mapping of the complete
        // AHCI MMIO range, and public accessors validate aligned register
        // offsets inside global or per-port register windows.
        unsafe { core::ptr::read_volatile(pointer) }
    }

    fn write_at(self, offset: u64, value: u32) {
        let pointer = (self.base + offset) as *mut u32;
        // SAFETY: same mapped-range and offset invariants as `read_at`; the
        // caller chooses a writable AHCI register through typed accessors.
        unsafe { core::ptr::write_volatile(pointer, value) };
    }
}

lazy_static! {
    static ref CONTROLLERS: Mutex<Vec<AhciController>> = Mutex::new(Vec::new());
}

pub fn init(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    physical_memory_offset: VirtAddr,
) {
    let candidates: Vec<AhciController> = pci::devices_by_class(PCI_CLASS_MASS_STORAGE)
        .into_iter()
        .filter(|device| device.subclass == PCI_SUBCLASS_SATA && device.prog_if == PCI_PROG_IF_AHCI)
        .filter_map(controller_from_pci)
        .collect();
    let mut controllers = Vec::new();
    for (index, controller) in candidates.into_iter().enumerate() {
        controllers.push(map_controller(
            controller,
            index,
            mapper,
            frame_allocator,
            physical_memory_offset,
        ));
    }

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
            "ABAR mapped; one SATA port rebased",
        );
        for controller in &controllers {
            crate::serial_println!(
                "[AHCI] {:02x}:{:02x}.{} vendor={:04x} device={:04x} abar={:#x} mmio_ready={} cap={:#010x} pi={:#010x} vs={:#010x}",
                controller.pci.bus,
                controller.pci.slot,
                controller.pci.function,
                controller.pci.vendor_id,
                controller.pci.device_id,
                controller.abar,
                controller.mmio_ready,
                controller.capabilities,
                controller.ports_implemented,
                controller.version
            );
            for port in controller.ports.iter().flatten() {
                crate::serial_println!(
                    "[AHCI] port={} active={} signature={:#010x} ssts={:#010x} rebased={} clb={:#x} fb={:#x} ctba={:#x} identify={} model={} blocks={} read_lba0={} boot_sig={:04x}",
                    port.index,
                    port.active,
                    port.signature,
                    port.sata_status,
                    port.rebased,
                    port.command_list,
                    port.received_fis,
                    port.command_table,
                    port.identify_ok,
                    port.model_str(),
                    port.block_count,
                    port.read_lba0_ok,
                    port.lba0_signature
                );
            }
        }
    }

    *CONTROLLERS.lock() = controllers;
    verify_block_device_path();
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
        mmio_base: 0,
        capabilities: 0,
        global_host_control: 0,
        ports_implemented: 0,
        version: 0,
        physical_memory_offset: 0,
        ports: [None; 32],
        rebased_port_count: 0,
    })
}

fn map_controller(
    mut controller: AhciController,
    index: usize,
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    physical_memory_offset: VirtAddr,
) -> AhciController {
    let Some(offset) = (index as u64).checked_mul(AHCI_MMIO_STRIDE) else {
        return controller;
    };
    let Some(mmio_base) = AHCI_MMIO_WINDOW.checked_add(offset) else {
        return controller;
    };
    if crate::memory::map_mmio_range(
        PhysAddr::new(controller.abar),
        VirtAddr::new(mmio_base),
        AHCI_MMIO_SIZE,
        mapper,
        frame_allocator,
    )
    .is_err()
    {
        return controller;
    }

    controller.mmio_base = mmio_base;
    controller.physical_memory_offset = physical_memory_offset.as_u64();
    let registers = AhciMmio::new(mmio_base);
    controller.capabilities = registers.global_read(REG_CAP);
    controller.global_host_control = registers.global_read(REG_GHC);
    controller.ports_implemented = registers.global_read(REG_PORTS_IMPLEMENTED);
    controller.version = registers.global_read(REG_VERSION);
    controller.mmio_ready = true;
    discover_and_rebase_ports(
        &mut controller,
        registers,
        frame_allocator,
        physical_memory_offset,
    );
    controller
}

fn discover_and_rebase_ports(
    controller: &mut AhciController,
    registers: AhciMmio,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    physical_memory_offset: VirtAddr,
) {
    for port_index in 0..32u8 {
        if controller.ports_implemented & (1u32 << port_index) == 0 {
            continue;
        }
        let signature = registers.port_read(port_index, PORT_SIG);
        let sata_status = registers.port_read(port_index, PORT_SSTS);
        let active = sata_device_active(signature, sata_status);
        let mut port = AhciPort {
            index: port_index,
            signature,
            sata_status,
            active,
            rebased: false,
            command_list: 0,
            received_fis: 0,
            command_table: 0,
            data_buffer: 0,
            identify_ok: false,
            model: [0; ATA_MODEL_LEN],
            model_len: 0,
            block_count: 0,
            read_lba0_ok: false,
            lba0_signature: 0,
        };
        if active {
            port.rebased = rebase_port(
                registers,
                port_index,
                &mut port,
                frame_allocator,
                physical_memory_offset,
            );
            if port.rebased {
                controller.rebased_port_count = controller.rebased_port_count.saturating_add(1);
                if let Ok(identify) = identify_device(
                    registers,
                    port_index,
                    physical_memory_offset,
                    port.command_list,
                    port.command_table,
                    port.data_buffer,
                ) {
                    port.identify_ok = true;
                    port.model = identify.model;
                    port.model_len = identify.model_len as u8;
                    port.block_count = identify.block_count;
                }
                if let Ok(sector) = read_dma_sector(
                    registers,
                    port_index,
                    physical_memory_offset,
                    port.command_list,
                    port.command_table,
                    port.data_buffer,
                    0,
                ) {
                    port.lba0_signature = boot_signature(&sector);
                    port.read_lba0_ok = port.lba0_signature == BOOT_SIGNATURE;
                }
            }
        }
        controller.ports[port_index as usize] = Some(port);
    }
}

fn sata_device_active(signature: u32, sata_status: u32) -> bool {
    let detection = sata_status & 0x0f;
    let interface_power = (sata_status >> 8) & 0x0f;
    signature == SATA_SIGNATURE && detection == 3 && interface_power == 1
}

#[cfg(test)]
fn port_base(mmio_base: u64, port_index: u8) -> u64 {
    mmio_base + PORT_BASE + u64::from(port_index) * PORT_STRIDE
}

fn rebase_port(
    registers: AhciMmio,
    port_index: u8,
    port: &mut AhciPort,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    physical_memory_offset: VirtAddr,
) -> bool {
    let Some(control_frame) = frame_allocator.allocate_frame() else {
        return false;
    };
    let Some(table_frame) = frame_allocator.allocate_frame() else {
        return false;
    };
    let Some(data_frame) = frame_allocator.allocate_frame() else {
        return false;
    };
    let control = control_frame.start_address().as_u64();
    let table = table_frame.start_address().as_u64();
    let data = data_frame.start_address().as_u64();

    zero_dma_page(physical_memory_offset, control);
    zero_dma_page(physical_memory_offset, table);
    zero_dma_page(physical_memory_offset, data);
    initialize_slot_zero(physical_memory_offset, control, table, data);

    if !stop_port(registers, port_index) {
        return false;
    }
    registers.port_write(port_index, PORT_CLB, control as u32);
    registers.port_write(port_index, PORT_CLBU, (control >> 32) as u32);
    let received_fis = control + RECEIVED_FIS_OFFSET;
    registers.port_write(port_index, PORT_FB, received_fis as u32);
    registers.port_write(port_index, PORT_FBU, (received_fis >> 32) as u32);
    registers.port_write(port_index, PORT_IS, u32::MAX);
    registers.port_write(port_index, PORT_SERR, u32::MAX);
    registers.port_write(port_index, PORT_IE, 0);
    if !start_port(registers, port_index) {
        return false;
    }

    port.command_list = control;
    port.received_fis = control + RECEIVED_FIS_OFFSET;
    port.command_table = table;
    port.data_buffer = data;
    true
}

fn zero_dma_page(physical_memory_offset: VirtAddr, physical: u64) {
    let pointer = (physical_memory_offset + physical).as_mut_ptr::<u8>();
    // SAFETY: the frame is exclusively owned DMA memory with exactly one page.
    unsafe { core::ptr::write_bytes(pointer, 0, Size4KiB::SIZE as usize) };
}

fn initialize_slot_zero(physical_memory_offset: VirtAddr, control: u64, table: u64, data: u64) {
    let header = AhciCommandHeader {
        flags: COMMAND_FIS_DWORDS,
        prdt_len: 1,
        transferred: 0,
        table_base: table as u32,
        table_base_upper: (table >> 32) as u32,
        reserved: [0; 4],
    };
    let prdt = AhciPrdtEntry {
        data_base: data as u32,
        data_base_upper: (data >> 32) as u32,
        reserved: 0,
        byte_count_interrupt: (PRDT_BYTE_COUNT - 1) | (1 << 31),
    };
    let header_pointer = (physical_memory_offset + control).as_mut_ptr::<AhciCommandHeader>();
    let prdt_pointer =
        (physical_memory_offset + table + PRDT_OFFSET as u64).as_mut_ptr::<AhciPrdtEntry>();
    // SAFETY: both pointers target exclusively owned, zeroed DMA frames and
    // satisfy AHCI command-header/table alignment requirements.
    unsafe {
        core::ptr::write(header_pointer, header);
        core::ptr::write(prdt_pointer, prdt);
    }
}

fn stop_port(registers: AhciMmio, port: u8) -> bool {
    let mut command = registers.port_read(port, PORT_CMD);
    command &= !PORT_CMD_ST;
    registers.port_write(port, PORT_CMD, command);
    if !wait_command_clear(registers, port, PORT_CMD_CR) {
        return false;
    }
    command &= !PORT_CMD_FRE;
    registers.port_write(port, PORT_CMD, command);
    wait_command_clear(registers, port, PORT_CMD_FR)
}

fn start_port(registers: AhciMmio, port: u8) -> bool {
    if !wait_command_clear(registers, port, PORT_CMD_CR) {
        return false;
    }
    let command = registers.port_read(port, PORT_CMD) | PORT_CMD_FRE | PORT_CMD_ST;
    registers.port_write(port, PORT_CMD, command);
    true
}

fn wait_command_clear(registers: AhciMmio, port: u8, mask: u32) -> bool {
    for _ in 0..PORT_POLL_LIMIT {
        if registers.port_read(port, PORT_CMD) & mask == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn identify_device(
    registers: AhciMmio,
    port: u8,
    physical_memory_offset: VirtAddr,
    command_list: u64,
    command_table: u64,
    data_buffer: u64,
) -> Result<IdentifyData, BlockError> {
    let mut fis = [0u8; 20];
    fis[0] = FIS_TYPE_REG_H2D;
    fis[1] = FIS_COMMAND;
    fis[2] = ATA_IDENTIFY_DEVICE;
    execute_slot_zero(
        registers,
        port,
        physical_memory_offset,
        command_list,
        command_table,
        data_buffer,
        &fis,
        true,
        false,
        None,
    )?;
    let identify = copy_dma_data(physical_memory_offset, data_buffer);
    let (model, model_len) = parse_identify_model(&identify);
    Ok(IdentifyData {
        model,
        model_len,
        block_count: parse_identify_block_count(&identify),
    })
}

fn read_dma_sector(
    registers: AhciMmio,
    port: u8,
    physical_memory_offset: VirtAddr,
    command_list: u64,
    command_table: u64,
    data_buffer: u64,
    lba: u64,
) -> Result<[u8; BLOCK_SIZE], BlockError> {
    if lba >= 1 << 48 {
        return Err(BlockError::OutOfRange);
    }
    let mut fis = [0u8; 20];
    fis[0] = FIS_TYPE_REG_H2D;
    fis[1] = FIS_COMMAND;
    fis[2] = ATA_READ_DMA_EXT;
    fis[4] = lba as u8;
    fis[5] = (lba >> 8) as u8;
    fis[6] = (lba >> 16) as u8;
    fis[7] = ATA_DEVICE_LBA;
    fis[8] = (lba >> 24) as u8;
    fis[9] = (lba >> 32) as u8;
    fis[10] = (lba >> 40) as u8;
    fis[12] = 1;
    execute_slot_zero(
        registers,
        port,
        physical_memory_offset,
        command_list,
        command_table,
        data_buffer,
        &fis,
        true,
        false,
        None,
    )?;
    Ok(copy_dma_data(physical_memory_offset, data_buffer))
}

fn write_dma_sector(
    registers: AhciMmio,
    port: u8,
    physical_memory_offset: VirtAddr,
    command_list: u64,
    command_table: u64,
    data_buffer: u64,
    lba: u64,
    data: &[u8; BLOCK_SIZE],
) -> Result<(), BlockError> {
    if lba >= 1 << 48 {
        return Err(BlockError::OutOfRange);
    }
    let mut fis = [0u8; 20];
    fis[0] = FIS_TYPE_REG_H2D;
    fis[1] = FIS_COMMAND;
    fis[2] = ATA_WRITE_DMA_EXT;
    fis[4] = lba as u8;
    fis[5] = (lba >> 8) as u8;
    fis[6] = (lba >> 16) as u8;
    fis[7] = ATA_DEVICE_LBA;
    fis[8] = (lba >> 24) as u8;
    fis[9] = (lba >> 32) as u8;
    fis[10] = (lba >> 40) as u8;
    fis[12] = 1;
    execute_slot_zero(
        registers,
        port,
        physical_memory_offset,
        command_list,
        command_table,
        data_buffer,
        &fis,
        true,
        true,
        Some(data),
    )
}

fn flush_cache(
    registers: AhciMmio,
    port: u8,
    physical_memory_offset: VirtAddr,
    command_list: u64,
    command_table: u64,
    data_buffer: u64,
) -> Result<(), BlockError> {
    let mut fis = [0u8; 20];
    fis[0] = FIS_TYPE_REG_H2D;
    fis[1] = FIS_COMMAND;
    fis[2] = ATA_FLUSH_CACHE_EXT;
    execute_slot_zero(
        registers,
        port,
        physical_memory_offset,
        command_list,
        command_table,
        data_buffer,
        &fis,
        false,
        false,
        None,
    )
}

fn execute_slot_zero(
    registers: AhciMmio,
    port: u8,
    physical_memory_offset: VirtAddr,
    command_list: u64,
    command_table: u64,
    data_buffer: u64,
    fis: &[u8; 20],
    transfers_data: bool,
    writes_to_device: bool,
    payload: Option<&[u8; BLOCK_SIZE]>,
) -> Result<(), BlockError> {
    if registers.port_read(port, PORT_CI) & 1 != 0 || registers.port_read(port, PORT_SACT) & 1 != 0
    {
        return Err(BlockError::DeviceFault);
    }
    for _ in 0..PORT_POLL_LIMIT {
        if registers.port_read(port, PORT_TFD) & (PORT_TFD_BSY | PORT_TFD_DRQ) == 0 {
            break;
        }
        core::hint::spin_loop();
    }
    if registers.port_read(port, PORT_TFD) & (PORT_TFD_BSY | PORT_TFD_DRQ) != 0 {
        return Err(BlockError::Timeout);
    }

    prepare_slot_zero_command(
        physical_memory_offset,
        command_list,
        command_table,
        data_buffer,
        fis,
        transfers_data,
        writes_to_device,
        payload,
    );
    registers.port_write(port, PORT_IS, u32::MAX);
    registers.port_write(port, PORT_SERR, u32::MAX);
    fence(Ordering::SeqCst);
    registers.port_write(port, PORT_CI, 1);

    for _ in 0..PORT_POLL_LIMIT {
        if registers.port_read(port, PORT_IS) & PORT_IS_TFES != 0 {
            return Err(BlockError::DeviceFault);
        }
        if registers.port_read(port, PORT_CI) & 1 == 0 {
            fence(Ordering::SeqCst);
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(BlockError::Timeout)
}

fn prepare_slot_zero_command(
    physical_memory_offset: VirtAddr,
    command_list: u64,
    command_table: u64,
    data_buffer: u64,
    fis: &[u8; 20],
    transfers_data: bool,
    writes_to_device: bool,
    payload: Option<&[u8; BLOCK_SIZE]>,
) {
    debug_assert!(!writes_to_device || payload.is_some());
    debug_assert!(payload.is_none() || transfers_data);
    let header = (physical_memory_offset + command_list).as_mut_ptr::<AhciCommandHeader>();
    let fis_pointer = (physical_memory_offset + command_table).as_mut_ptr::<u8>();
    let data_pointer = (physical_memory_offset + data_buffer).as_mut_ptr::<u8>();
    // SAFETY: header and command_table point to the exclusively owned slot-0
    // DMA structures. The FIS fits at the table start, the header fields are
    // naturally aligned, and an optional sector payload fits in the DMA page.
    unsafe {
        (*header).flags = COMMAND_FIS_DWORDS
            | if writes_to_device {
                COMMAND_HEADER_WRITE
            } else {
                0
            };
        (*header).prdt_len = u16::from(transfers_data);
        (*header).transferred = 0;
        core::ptr::copy_nonoverlapping(fis.as_ptr(), fis_pointer, fis.len());
        if let Some(payload) = payload {
            core::ptr::copy_nonoverlapping(payload.as_ptr(), data_pointer, payload.len());
        }
    }
}

fn copy_dma_data(physical_memory_offset: VirtAddr, data_buffer: u64) -> [u8; BLOCK_SIZE] {
    let mut data = [0u8; BLOCK_SIZE];
    let pointer = (physical_memory_offset + data_buffer).as_ptr::<u8>();
    // SAFETY: data_buffer is an exclusively owned page-sized DMA frame. The
    // command has completed and the fence above makes its first 512 bytes stable.
    unsafe { core::ptr::copy_nonoverlapping(pointer, data.as_mut_ptr(), data.len()) };
    data
}

fn boot_signature(sector: &[u8; 512]) -> u16 {
    u16::from_be_bytes([
        sector[BOOT_SIGNATURE_OFFSET],
        sector[BOOT_SIGNATURE_OFFSET + 1],
    ])
}

fn parse_identify_model(identify: &[u8; 512]) -> ([u8; ATA_MODEL_LEN], usize) {
    let mut model = [b' '; ATA_MODEL_LEN];
    for index in (0..ATA_MODEL_LEN).step_by(2) {
        let first = identify[ATA_MODEL_OFFSET + index + 1];
        let second = identify[ATA_MODEL_OFFSET + index];
        model[index] = printable_ascii(first);
        model[index + 1] = printable_ascii(second);
    }
    let len = model
        .iter()
        .rposition(|byte| *byte != b' ')
        .map_or(0, |index| index + 1);
    (model, len)
}

fn parse_identify_block_count(identify: &[u8; BLOCK_SIZE]) -> u64 {
    let supports_lba48 = identify_word(identify, 83) & (1 << 10) != 0;
    if supports_lba48 {
        let mut blocks = 0u64;
        for word in (100..=103).rev() {
            blocks = (blocks << 16) | u64::from(identify_word(identify, word));
        }
        if blocks != 0 {
            return blocks;
        }
    }
    u64::from(identify_word(identify, 60)) | (u64::from(identify_word(identify, 61)) << 16)
}

fn identify_word(identify: &[u8; BLOCK_SIZE], word: usize) -> u16 {
    let offset = word * 2;
    u16::from_le_bytes([identify[offset], identify[offset + 1]])
}

fn printable_ascii(byte: u8) -> u8 {
    if (b' '..=b'~').contains(&byte) {
        byte
    } else if byte == 0 {
        b' '
    } else {
        b'?'
    }
}

impl AhciPort {
    fn model_str(&self) -> &str {
        core::str::from_utf8(&self.model[..self.model_len as usize]).unwrap_or("?")
    }
}

impl AhciBlockDevice {
    pub fn primary() -> Result<Self, BlockError> {
        Self::nth(0)
    }

    pub fn nth(mut target: usize) -> Result<Self, BlockError> {
        let controllers = CONTROLLERS.lock();
        for (controller_index, controller) in controllers.iter().enumerate() {
            for port in controller.ports.iter().flatten() {
                if port.rebased && port.identify_ok && port.block_count != 0 {
                    if target != 0 {
                        target -= 1;
                        continue;
                    }
                    return Ok(Self {
                        controller_index,
                        port_index: port.index,
                        blocks: port.block_count,
                    });
                }
            }
        }
        Err(BlockError::NoDevice)
    }
}

impl BlockDevice for AhciBlockDevice {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(
        &mut self,
        block_index: u64,
        buffer: &mut [u8; BLOCK_SIZE],
    ) -> Result<(), BlockError> {
        if block_index >= self.blocks {
            return Err(BlockError::OutOfRange);
        }
        let controllers = CONTROLLERS.lock();
        let controller = controllers
            .get(self.controller_index)
            .ok_or(BlockError::NoDevice)?;
        let port = controller.ports[self.port_index as usize].ok_or(BlockError::NoDevice)?;
        if !port.rebased || !port.active {
            return Err(BlockError::NoDevice);
        }
        *buffer = read_dma_sector(
            AhciMmio::new(controller.mmio_base),
            self.port_index,
            VirtAddr::new(controller.physical_memory_offset),
            port.command_list,
            port.command_table,
            port.data_buffer,
            block_index,
        )?;
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
        let controllers = CONTROLLERS.lock();
        let controller = controllers
            .get(self.controller_index)
            .ok_or(BlockError::NoDevice)?;
        let port = controller.ports[self.port_index as usize].ok_or(BlockError::NoDevice)?;
        if !port.rebased || !port.active {
            return Err(BlockError::NoDevice);
        }
        let registers = AhciMmio::new(controller.mmio_base);
        let physical_memory_offset = VirtAddr::new(controller.physical_memory_offset);
        write_dma_sector(
            registers,
            self.port_index,
            physical_memory_offset,
            port.command_list,
            port.command_table,
            port.data_buffer,
            block_index,
            buffer,
        )?;
        flush_cache(
            registers,
            self.port_index,
            physical_memory_offset,
            port.command_list,
            port.command_table,
            port.data_buffer,
        )
    }
}

fn verify_block_device_path() {
    let result = (|| {
        let mut disk = AhciBlockDevice::primary()?;
        let blocks = disk.block_count();
        let mut sector = [0u8; BLOCK_SIZE];
        disk.read_block(0, &mut sector)?;
        let signature = boot_signature(&sector);
        if signature != BOOT_SIGNATURE {
            return Err(BlockError::DeviceFault);
        }
        Ok((blocks, signature))
    })();
    match result {
        Ok((blocks, signature)) => {
            crate::drivers::status::report(
                "ahci-block",
                crate::drivers::status::DriverState::Ready,
                "synchronized DMA BlockDevice with flush",
            );
            crate::serial_println!(
                "[AHCI-BLOCK] ready blocks={} read_lba0=true boot_sig={:04x} writable=true flush=true",
                blocks,
                signature
            );
        }
        Err(error) => {
            crate::drivers::status::report(
                "ahci-block",
                crate::drivers::status::DriverState::Missing,
                "DMA BlockDevice unavailable",
            );
            crate::serial_println!("[AHCI-BLOCK] unavailable error={:?}", error);
        }
    }
}

#[cfg(feature = "ahci-write-test")]
pub fn run_write_test() {
    const MAGIC: &[u8] = b"VANTARA_AHCI_WRITE_V1";
    let result = (|| {
        let mut disk = AhciBlockDevice::primary()?;
        let lba = disk
            .block_count()
            .checked_sub(1)
            .ok_or(BlockError::NoDevice)?;
        let mut sector = [0u8; BLOCK_SIZE];
        disk.read_block(lba, &mut sector)?;
        if sector.starts_with(MAGIC) {
            return Ok(("verify", lba, sector_checksum(&sector)));
        }
        let mut marker = [0u8; BLOCK_SIZE];
        marker[..MAGIC.len()].copy_from_slice(MAGIC);
        for (index, byte) in marker[MAGIC.len()..].iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(17).wrapping_add(0x5a);
        }
        disk.write_block(lba, &marker)?;
        let mut readback = [0u8; BLOCK_SIZE];
        disk.read_block(lba, &mut readback)?;
        if readback != marker {
            return Err(BlockError::DeviceFault);
        }
        Ok(("write", lba, sector_checksum(&readback)))
    })();
    match result {
        Ok((phase, lba, checksum)) => {
            crate::serial_println!(
                "[AHCI-WRITE-TEST] phase={} lba={} checksum={} persisted=true flush=true",
                phase,
                lba,
                checksum
            );
        }
        Err(error) => {
            crate::serial_println!("[AHCI-WRITE-TEST] failed error={:?}", error);
        }
    }
}

#[cfg(feature = "ahci-write-test")]
fn sector_checksum(sector: &[u8; BLOCK_SIZE]) -> u64 {
    sector
        .iter()
        .fold(0u64, |sum, byte| sum.wrapping_add(u64::from(*byte)))
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
    writer.write_str(
        "BDF       VENDOR:DEVICE ABAR               CAP        PI         VS         STATE\n",
    );
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
        writer.write_byte(b' ');
        writer.write_hex_u32(controller.capabilities);
        writer.write_byte(b' ');
        writer.write_hex_u32(controller.ports_implemented);
        writer.write_byte(b' ');
        writer.write_hex_u32(controller.version);
        writer.write_str(if controller.mmio_ready {
            " ready\n"
        } else {
            " discovery\n"
        });
        for port in controller.ports.iter().flatten() {
            writer.write_str("  port ");
            writer.write_dec(port.index as u64);
            writer.write_str(" active=");
            writer.write_dec(port.active as u64);
            writer.write_str(" rebased=");
            writer.write_dec(port.rebased as u64);
            writer.write_str(" sig=");
            writer.write_hex_u32(port.signature);
            writer.write_str(" clb=0x");
            writer.write_hex_u64(port.command_list);
            writer.write_str(" identify=");
            writer.write_dec(port.identify_ok as u64);
            writer.write_str(" model=");
            writer.write_str(port.model_str());
            writer.write_str(" blocks=");
            writer.write_dec(port.block_count);
            writer.write_str(" read_lba0=");
            writer.write_dec(port.read_lba0_ok as u64);
            writer.write_str(" boot_sig=");
            writer.write_hex_u16(port.lba0_signature);
            writer.write_byte(b'\n');
        }
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

    fn write_hex_u32(&mut self, value: u32) {
        for shift in (0..8).rev() {
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
    use super::{
        ATA_MODEL_LEN, ATA_MODEL_OFFSET, AhciBlockDevice, BOOT_SIGNATURE, BOOT_SIGNATURE_OFFSET,
        BufferWriter, SATA_SIGNATURE, boot_signature, parse_identify_block_count,
        parse_identify_model, port_base, sata_device_active,
    };
    use crate::storage::block::{BLOCK_SIZE, BlockDevice, BlockError};

    #[test_case]
    fn formats_full_width_abar() {
        let mut out = [0u8; 32];
        let mut writer = BufferWriter::new(&mut out);
        writer.write_hex_u64(0x1234_abcd);
        let len = writer.len();
        drop(writer);
        assert_eq!(&out[..len], b"000000001234abcd");
    }

    #[test_case]
    fn detects_only_active_sata_links() {
        assert!(sata_device_active(SATA_SIGNATURE, 0x103));
        assert!(!sata_device_active(SATA_SIGNATURE, 0x003));
        assert!(!sata_device_active(0xeb14_0101, 0x103));
    }

    #[test_case]
    fn computes_port_register_windows() {
        assert_eq!(port_base(0xffff_9000_0000_0000, 0), 0xffff_9000_0000_0100);
        assert_eq!(port_base(0xffff_9000_0000_0000, 31), 0xffff_9000_0000_1080);
    }

    #[test_case]
    fn parses_word_swapped_identify_model() {
        let mut identify = [0u8; 512];
        let mut padded = [b' '; ATA_MODEL_LEN];
        padded[..13].copy_from_slice(b"QEMU HARDDISK");
        for index in (0..ATA_MODEL_LEN).step_by(2) {
            identify[ATA_MODEL_OFFSET + index] = padded[index + 1];
            identify[ATA_MODEL_OFFSET + index + 1] = padded[index];
        }
        let (model, len) = parse_identify_model(&identify);
        assert_eq!(&model[..len], b"QEMU HARDDISK");
    }

    #[test_case]
    fn parses_boot_signature_from_sector_zero() {
        let mut sector = [0u8; 512];
        sector[BOOT_SIGNATURE_OFFSET..].copy_from_slice(&[0x55, 0xaa]);
        assert_eq!(boot_signature(&sector), BOOT_SIGNATURE);
    }

    #[test_case]
    fn parses_lba48_block_count() {
        let mut identify = [0u8; BLOCK_SIZE];
        identify[83 * 2..83 * 2 + 2].copy_from_slice(&(1u16 << 10).to_le_bytes());
        let blocks = 0x0000_0001_2345_6789u64;
        for (index, word) in (100..=103).enumerate() {
            identify[word * 2..word * 2 + 2]
                .copy_from_slice(&((blocks >> (index * 16)) as u16).to_le_bytes());
        }
        assert_eq!(parse_identify_block_count(&identify), blocks);
    }

    #[test_case]
    fn block_device_rejects_range_overflow() {
        let mut disk = AhciBlockDevice {
            controller_index: 0,
            port_index: 0,
            blocks: 8,
        };
        let block = [0u8; BLOCK_SIZE];
        assert_eq!(disk.write_block(8, &block), Err(BlockError::OutOfRange));
        let mut out = [0u8; BLOCK_SIZE];
        assert_eq!(disk.read_block(8, &mut out), Err(BlockError::OutOfRange));
    }
}
