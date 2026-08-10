use alloc::vec::Vec;
use core::ptr::{read_volatile, write_bytes, write_volatile};
use lazy_static::lazy_static;
use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{FrameAllocator, Mapper, Size4KiB},
};

use crate::drivers::pci::{self, PciBar, PciDevice};
use crate::sync::PreemptMutex as Mutex;

const USB_CLASS: u8 = 0x0c;
const USB_SUBCLASS: u8 = 0x03;
const XHCI_PROG_IF: u8 = 0x30;
const MMIO_WINDOW: u64 = 0xffff_9400_0000_0000;
const MMIO_STRIDE: u64 = 0x4000;
const MMIO_SIZE: u64 = 0x4000;
const HCSPARAMS1: u64 = 0x04;
const DBOFF: u64 = 0x14;
const RTSOFF: u64 = 0x18;
const USBCMD: u64 = 0x00;
const USBSTS: u64 = 0x04;
const CRCR: u64 = 0x18;
const DCBAAP: u64 = 0x30;
const CONFIG: u64 = 0x38;
const PORTSC_BASE: u64 = 0x400;
const RUNTIME_INTERRUPTER0: u64 = 0x20;
const IMAN: u64 = 0x00;
const ERSTSZ: u64 = 0x08;
const ERSTBA: u64 = 0x10;
const ERDP: u64 = 0x18;
const CMD_RUN: u32 = 1;
const CMD_RESET: u32 = 1 << 1;
const STS_HALTED: u32 = 1;
const TRB_CYCLE: u32 = 1;
const TRB_TYPE_LINK: u32 = 6 << 10;
const TRB_TYPE_ENABLE_SLOT: u32 = 9 << 10;
const TRB_TYPE_ADDRESS_DEVICE: u32 = 11 << 10;
const TRB_TYPE_CONFIGURE_ENDPOINT: u32 = 12 << 10;
const TRB_TYPE_SETUP_STAGE: u32 = 2 << 10;
const TRB_TYPE_NORMAL: u32 = 1 << 10;
const TRB_TYPE_DATA_STAGE: u32 = 3 << 10;
const TRB_TYPE_STATUS_STAGE: u32 = 4 << 10;
const TRB_TYPE_TRANSFER_EVENT: u32 = 32;
const TRB_TYPE_COMMAND_COMPLETION: u32 = 33;
const WAIT_LIMIT: usize = 1_000_000;

#[repr(C, align(16))]
struct Trb {
    parameter: u64,
    status: u32,
    control: u32,
}

#[repr(C, align(16))]
struct EventRingSegment {
    base: u64,
    size: u16,
    reserved: [u8; 6],
}

#[derive(Debug, Clone, Copy)]
struct EndpointProfile {
    address: u8,
    transfer_type: u8,
    max_packet: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XhciController {
    pub pci: PciDevice,
    pub bar0: u64,
    pub mmio_base: u64,
    pub cap_length: u8,
    pub version: u16,
    pub max_slots: u8,
    pub port_count: u8,
    pub doorbell_offset: u32,
    pub runtime_offset: u32,
    pub command_ring: u64,
    pub transfer_ring: u64,
    pub event_ring: u64,
    pub rings_ready: bool,
    pub enabled_slot: u8,
    pub addressed_port: u8,
    pub command_completions: u32,
    physical_memory_offset: u64,
    dcbaa: u64,
    input_context: u64,
    output_context: u64,
    transfer_data: u64,
    bulk_out_ring: u64,
    bulk_in_ring: u64,
    command_enqueue: u16,
    command_cycle: bool,
    event_dequeue: u16,
    event_cycle: bool,
    transfer_enqueue: u16,
    context_size: u16,
    addressed_speed: u8,
    input_endpoint_dci: u8,
    input_max_packet: u16,
    input_transfer_type: u8,
    input_enqueue: u16,
    input_reports: u64,
}

lazy_static! {
    static ref CONTROLLERS: Mutex<Vec<XhciController>> = Mutex::new(Vec::new());
}

pub fn init(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    physical_memory_offset: VirtAddr,
) {
    let devices: Vec<_> = pci::devices_by_class(USB_CLASS)
        .into_iter()
        .filter(|device| device.subclass == USB_SUBCLASS && device.prog_if == XHCI_PROG_IF)
        .collect();
    if devices.is_empty() {
        crate::drivers::status::report(
            "usb-xhci",
            crate::drivers::status::DriverState::Missing,
            "controller not found",
        );
        crate::serial_println!("[XHCI] no USB 3.x controller detected");
        return;
    }
    let mut controllers = Vec::new();
    for (index, device) in devices.into_iter().enumerate() {
        let Some(bar0) = memory_bar(device) else {
            continue;
        };
        let Some(mmio_base) = MMIO_WINDOW.checked_add(index as u64 * MMIO_STRIDE) else {
            continue;
        };
        pci::enable_memory_and_bus_master(device);
        if crate::memory::map_mmio_range(
            PhysAddr::new(bar0),
            VirtAddr::new(mmio_base),
            MMIO_SIZE,
            mapper,
            frame_allocator,
        )
        .is_err()
        {
            crate::serial_println!("[XHCI] MMIO map failed BAR0={:#x}", bar0);
            continue;
        }
        let mut controller = read_controller(device, bar0, mmio_base);
        match controller.initialize_rings(frame_allocator, physical_memory_offset) {
            Ok(()) => {
                crate::serial_println!(
                    "[XHCI] rings ready cmd={:#x} transfer={:#x} event={:#x}",
                    controller.command_ring,
                    controller.transfer_ring,
                    controller.event_ring
                );
            }
            Err(error) => {
                crate::serial_println!("[XHCI] ring setup unavailable: {}", error);
            }
        }
        match controller.enable_and_address_first_device() {
            Ok(Some((slot, port))) => {
                crate::serial_println!(
                    "[XHCI] Address Device complete slot={} port={}",
                    slot,
                    port
                );
            }
            Ok(None) => {
                crate::serial_println!("[XHCI] no connected root-port device to address");
            }
            Err(error) => {
                crate::serial_println!("[XHCI] command path failed: {}", error);
            }
        }
        crate::serial_println!(
            "[XHCI] {:02x}:{:02x}.{} bar0={:#x} caplen={} version={:04x} slots={} ports={} dboff={:#x} rtsoff={:#x}",
            device.bus,
            device.slot,
            device.function,
            bar0,
            controller.cap_length,
            controller.version,
            controller.max_slots,
            controller.port_count,
            controller.doorbell_offset,
            controller.runtime_offset
        );
        controllers.push(controller);
    }
    let state = if controllers.is_empty() {
        crate::drivers::status::DriverState::Error
    } else {
        crate::drivers::status::DriverState::Ready
    };
    crate::drivers::status::report(
        "usb-xhci",
        state,
        "USB 3.x command, event, and transfer rings initialized",
    );
    *CONTROLLERS.lock() = controllers;
}

fn memory_bar(device: PciDevice) -> Option<u64> {
    match pci::read_bar_info(device, 0) {
        PciBar::Memory32 { address, .. } | PciBar::Memory64 { address, .. } if address != 0 => {
            Some(address)
        }
        _ => None,
    }
}

fn read_controller(pci: PciDevice, bar0: u64, mmio_base: u64) -> XhciController {
    // SAFETY: the controller's mapped capability window is owned during init.
    unsafe {
        let cap = mmio_base as *const u8;
        let cap_length = read_volatile(cap);
        let version = read_volatile(cap.add(2).cast::<u16>());
        let params = read_volatile(cap.add(HCSPARAMS1 as usize).cast::<u32>());
        let doorbell_offset = read_volatile(cap.add(DBOFF as usize).cast::<u32>()) & !3;
        let runtime_offset = read_volatile(cap.add(RTSOFF as usize).cast::<u32>()) & !0x1f;
        XhciController {
            pci,
            bar0,
            mmio_base,
            cap_length,
            version,
            max_slots: (params & 0xff) as u8,
            port_count: ((params >> 24) & 0xff) as u8,
            doorbell_offset,
            runtime_offset,
            command_ring: 0,
            transfer_ring: 0,
            event_ring: 0,
            rings_ready: false,
            enabled_slot: 0,
            addressed_port: 0,
            command_completions: 0,
            physical_memory_offset: 0,
            dcbaa: 0,
            input_context: 0,
            output_context: 0,
            transfer_data: 0,
            bulk_out_ring: 0,
            bulk_in_ring: 0,
            command_enqueue: 0,
            command_cycle: true,
            event_dequeue: 0,
            event_cycle: true,
            transfer_enqueue: 0,
            context_size: if read_volatile(cap.add(0x10).cast::<u32>()) & (1 << 2) != 0 {
                64
            } else {
                32
            },
            addressed_speed: 0,
            input_endpoint_dci: 0,
            input_max_packet: 0,
            input_transfer_type: 0,
            input_enqueue: 0,
            input_reports: 0,
        }
    }
}

impl XhciController {
    fn op_ptr(&self, offset: u64) -> *mut u32 {
        (self.mmio_base + u64::from(self.cap_length) + offset) as *mut u32
    }
    fn op64_ptr(&self, offset: u64) -> *mut u64 {
        self.op_ptr(offset).cast()
    }
    fn read_op(&self, offset: u64) -> u32 {
        unsafe { read_volatile(self.op_ptr(offset)) }
    }
    fn write_op(&self, offset: u64, value: u32) {
        unsafe { write_volatile(self.op_ptr(offset), value) }
    }
    fn wait_op(&self, mask: u32, expected: u32) -> bool {
        for _ in 0..WAIT_LIMIT {
            if self.read_op(USBSTS) & mask == expected {
                return true;
            }
            core::hint::spin_loop();
        }
        false
    }

    fn initialize_rings(
        &mut self,
        allocator: &mut impl FrameAllocator<Size4KiB>,
        offset: VirtAddr,
    ) -> Result<(), &'static str> {
        self.write_op(USBCMD, self.read_op(USBCMD) & !CMD_RUN);
        if !self.wait_op(STS_HALTED, STS_HALTED) {
            return Err("controller did not halt");
        }
        self.write_op(USBCMD, CMD_RESET);
        for _ in 0..WAIT_LIMIT {
            if self.read_op(USBCMD) & CMD_RESET == 0 && self.read_op(USBSTS) & 0x800 == 0 {
                break;
            }
            core::hint::spin_loop();
        }
        if self.read_op(USBCMD) & CMD_RESET != 0 {
            return Err("controller reset timed out");
        }
        let dcbaa = alloc_frame(allocator)?;
        let command = alloc_frame(allocator)?;
        let transfer = alloc_frame(allocator)?;
        let event = alloc_frame(allocator)?;
        let erst = alloc_frame(allocator)?;
        let input_context = alloc_frame(allocator)?;
        let output_context = alloc_frame(allocator)?;
        let transfer_data = alloc_frame(allocator)?;
        let bulk_out_ring = alloc_frame(allocator)?;
        let bulk_in_ring = alloc_frame(allocator)?;
        // SAFETY: each is a distinct allocated DMA page addressed through the
        // bootloader physical-memory mapping; xHCI consumes initialized TRBs.
        unsafe {
            for frame in [
                dcbaa,
                command,
                transfer,
                event,
                erst,
                input_context,
                output_context,
                transfer_data,
                bulk_out_ring,
                bulk_in_ring,
            ] {
                write_bytes((offset + frame).as_mut_ptr::<u8>(), 0, 4096);
            }
            let command_trbs = (offset + command).as_mut_ptr::<Trb>();
            write_volatile(&mut (*command_trbs.add(255)).parameter, command);
            write_volatile(
                &mut (*command_trbs.add(255)).control,
                TRB_TYPE_LINK | (1 << 1) | TRB_CYCLE,
            );
            let transfer_trbs = (offset + transfer).as_mut_ptr::<Trb>();
            write_volatile(&mut (*transfer_trbs.add(255)).parameter, transfer);
            write_volatile(
                &mut (*transfer_trbs.add(255)).control,
                TRB_TYPE_LINK | (1 << 1) | TRB_CYCLE,
            );
            let segment = (offset + erst).as_mut_ptr::<EventRingSegment>();
            write_volatile(&mut (*segment).base, event);
            write_volatile(&mut (*segment).size, 256);
            write_volatile(self.op64_ptr(CRCR), command | TRB_CYCLE as u64);
            write_volatile(self.op64_ptr(DCBAAP), dcbaa);
            write_volatile(self.op_ptr(CONFIG), u32::from(self.max_slots));
            let interrupter =
                (self.mmio_base + u64::from(self.runtime_offset) + RUNTIME_INTERRUPTER0) as *mut u8;
            write_volatile(interrupter.add(ERSTSZ as usize).cast::<u32>(), 1);
            write_volatile(interrupter.add(ERSTBA as usize).cast::<u64>(), erst);
            write_volatile(interrupter.add(ERDP as usize).cast::<u64>(), event);
            write_volatile(interrupter.add(IMAN as usize).cast::<u32>(), 0);
        }
        self.write_op(USBCMD, CMD_RUN);
        if !self.wait_op(STS_HALTED, 0) {
            return Err("controller did not run");
        }
        self.command_ring = command;
        self.transfer_ring = transfer;
        self.event_ring = event;
        self.physical_memory_offset = offset.as_u64();
        self.dcbaa = dcbaa;
        self.input_context = input_context;
        self.output_context = output_context;
        self.transfer_data = transfer_data;
        self.bulk_out_ring = bulk_out_ring;
        self.bulk_in_ring = bulk_in_ring;
        self.rings_ready = true;
        Ok(())
    }

    fn doorbell_ptr(&self, index: u8) -> *mut u32 {
        (self.mmio_base + u64::from(self.doorbell_offset) + u64::from(index) * 4) as *mut u32
    }

    fn interrupter_ptr(&self, offset: u64) -> *mut u8 {
        (self.mmio_base + u64::from(self.runtime_offset) + RUNTIME_INTERRUPTER0 + offset) as *mut u8
    }

    fn submit_command(
        &mut self,
        parameter: u64,
        status: u32,
        control: u32,
    ) -> Result<(u8, u8), &'static str> {
        if !self.rings_ready || self.command_enqueue >= 255 {
            return Err("xHCI command ring unavailable");
        }
        let offset = VirtAddr::new(self.physical_memory_offset);
        let trbs = (offset + self.command_ring).as_mut_ptr::<Trb>();
        let cycle = u32::from(self.command_cycle);
        // SAFETY: enqueue points inside the exclusively owned command page;
        // the cycle bit is published last before ringing doorbell zero.
        unsafe {
            let trb = trbs.add(usize::from(self.command_enqueue));
            write_volatile(&mut (*trb).parameter, parameter);
            write_volatile(&mut (*trb).status, status);
            write_volatile(&mut (*trb).control, control | cycle);
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            write_volatile(self.doorbell_ptr(0), 0);
        }
        self.command_enqueue += 1;
        let result = self.wait_event(TRB_TYPE_COMMAND_COMPLETION)?;
        self.command_completions = self.command_completions.saturating_add(1);
        Ok(result)
    }

    fn wait_event(&mut self, expected_type: u32) -> Result<(u8, u8), &'static str> {
        let offset = VirtAddr::new(self.physical_memory_offset);
        let events = (offset + self.event_ring).as_ptr::<Trb>();
        for _ in 0..WAIT_LIMIT {
            // SAFETY: dequeue is bounded to the initialized event segment and
            // the controller publishes the cycle bit after the event payload.
            let event = unsafe { &*events.add(usize::from(self.event_dequeue)) };
            let control = unsafe { read_volatile(&event.control) };
            if control & 1 == u32::from(self.event_cycle) {
                let event_type = (control >> 10) & 0x3f;
                let status = unsafe { read_volatile(&event.status) };
                let completion = (status >> 24) as u8;
                let slot = (control >> 24) as u8;
                self.event_dequeue += 1;
                if self.event_dequeue == 256 {
                    self.event_dequeue = 0;
                    self.event_cycle = !self.event_cycle;
                }
                let dequeue = self.event_ring
                    + u64::from(self.event_dequeue) * core::mem::size_of::<Trb>() as u64;
                // SAFETY: ERDP belongs to interrupter zero; EHB acknowledges
                // the consumed event while preserving the dequeue alignment.
                unsafe {
                    write_volatile(self.interrupter_ptr(ERDP).cast::<u64>(), dequeue | (1 << 3))
                };
                if event_type != expected_type {
                    continue;
                }
                if completion != 1
                    && !(expected_type == TRB_TYPE_TRANSFER_EVENT && completion == 13)
                {
                    return Err("xHCI event completed with failure");
                }
                return Ok((slot, completion));
            }
            core::hint::spin_loop();
        }
        Err("xHCI command completion timed out")
    }

    fn enable_and_address_first_device(&mut self) -> Result<Option<(u8, u8)>, &'static str> {
        let Some((port, speed)) = self.reset_first_connected_port()? else {
            return Ok(None);
        };
        let (slot, _) = self.submit_command(0, 0, TRB_TYPE_ENABLE_SLOT)?;
        if slot == 0 {
            return Err("Enable Slot returned slot zero");
        }
        crate::serial_println!("[XHCI] Enable Slot complete slot={}", slot);
        self.prepare_address_context(slot, port, speed)?;
        self.submit_command(
            self.input_context,
            0,
            TRB_TYPE_ADDRESS_DEVICE | (u32::from(slot) << 24),
        )?;
        self.enabled_slot = slot;
        self.addressed_port = port;
        self.addressed_speed = speed;
        let (configuration, endpoints) = self.fetch_endpoint_descriptors(slot)?;
        self.control_transfer(slot, [0, 9, configuration, 0, 0, 0, 0, 0], None)?;
        self.configure_endpoints(slot, port, speed, &endpoints)?;
        crate::serial_println!(
            "[XHCI] Configure Endpoint complete slot={} endpoints={}",
            slot,
            endpoints.len()
        );
        if self.input_transfer_type == 3 {
            crate::serial_println!(
                "[XHCI-HID] polling ready slot={} endpoint={} max-packet={}",
                slot,
                self.input_endpoint_dci,
                self.input_max_packet
            );
        }
        Ok(Some((slot, port)))
    }

    fn reset_first_connected_port(&self) -> Result<Option<(u8, u8)>, &'static str> {
        for port_index in 0..self.port_count {
            let register = PORTSC_BASE + u64::from(port_index) * 0x10;
            let status = self.read_op(register);
            if status & 1 == 0 {
                continue;
            }
            self.write_op(register, status | (1 << 4));
            for _ in 0..WAIT_LIMIT {
                let reset = self.read_op(register);
                if reset & (1 << 4) == 0 {
                    if reset & (1 << 1) == 0 {
                        return Err("xHCI root port did not enable");
                    }
                    return Ok(Some((port_index + 1, ((reset >> 10) & 0x0f) as u8)));
                }
                core::hint::spin_loop();
            }
            return Err("xHCI root-port reset timed out");
        }
        Ok(None)
    }

    fn control_transfer(
        &mut self,
        slot: u8,
        setup: [u8; 8],
        input: Option<&mut [u8]>,
    ) -> Result<usize, &'static str> {
        let input_length = input.as_ref().map_or(0, |bytes| bytes.len());
        if input_length > 4096 || self.transfer_enqueue > 251 {
            return Err("xHCI control transfer too large");
        }
        let offset = VirtAddr::new(self.physical_memory_offset);
        let ring = (offset + self.transfer_ring).as_mut_ptr::<Trb>();
        let data = (offset + self.transfer_data).as_mut_ptr::<u8>();
        let setup_parameter = u64::from_le_bytes(setup);
        let start = usize::from(self.transfer_enqueue);
        // SAFETY: three consecutive TRBs fit before the reserved Link TRB;
        // setup immediate-data and optional DMA buffer are initialized first.
        unsafe {
            write_bytes(data, 0, 4096);
            let setup_trb = ring.add(start);
            write_volatile(&mut (*setup_trb).parameter, setup_parameter);
            write_volatile(&mut (*setup_trb).status, 8);
            write_volatile(
                &mut (*setup_trb).control,
                TRB_TYPE_SETUP_STAGE
                    | (1 << 6)
                    | (if input_length == 0 { 0 } else { 3 << 16 })
                    | TRB_CYCLE,
            );
            let mut status_index = start + 1;
            if input_length != 0 {
                let data_trb = ring.add(status_index);
                write_volatile(&mut (*data_trb).parameter, self.transfer_data);
                write_volatile(&mut (*data_trb).status, input_length as u32);
                write_volatile(
                    &mut (*data_trb).control,
                    TRB_TYPE_DATA_STAGE | (1 << 16) | TRB_CYCLE,
                );
                status_index += 1;
            }
            let status_trb = ring.add(status_index);
            write_volatile(&mut (*status_trb).parameter, 0);
            write_volatile(&mut (*status_trb).status, 0);
            write_volatile(
                &mut (*status_trb).control,
                TRB_TYPE_STATUS_STAGE | (u32::from(input_length == 0) << 16) | (1 << 5) | TRB_CYCLE,
            );
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            write_volatile(self.doorbell_ptr(slot), 1);
            self.transfer_enqueue = (status_index + 1) as u16;
        }
        self.wait_event(TRB_TYPE_TRANSFER_EVENT)?;
        if let Some(destination) = input {
            // SAFETY: successful completion placed at most the requested bytes
            // in the dedicated transfer data page.
            unsafe {
                core::ptr::copy_nonoverlapping(data, destination.as_mut_ptr(), destination.len())
            };
        }
        Ok(input_length)
    }

    fn fetch_endpoint_descriptors(
        &mut self,
        slot: u8,
    ) -> Result<(u8, Vec<EndpointProfile>), &'static str> {
        let mut device = [0u8; 18];
        self.control_transfer(slot, [0x80, 6, 0, 1, 0, 0, 18, 0], Some(&mut device))?;
        if device[0] != 18 || device[1] != 1 {
            return Err("invalid xHCI device descriptor");
        }
        let mut header = [0u8; 9];
        self.control_transfer(slot, [0x80, 6, 0, 2, 0, 0, 9, 0], Some(&mut header))?;
        let total = usize::from(u16::from_le_bytes([header[2], header[3]]));
        if !(9..=4096).contains(&total) || header[5] == 0 {
            return Err("invalid xHCI configuration descriptor");
        }
        let mut configuration = alloc::vec![0u8; total];
        let length = (total as u16).to_le_bytes();
        self.control_transfer(
            slot,
            [0x80, 6, 0, 2, 0, 0, length[0], length[1]],
            Some(&mut configuration),
        )?;
        let mut endpoints = Vec::new();
        let mut index = 0usize;
        while index + 2 <= configuration.len() {
            let descriptor_length = usize::from(configuration[index]);
            if descriptor_length < 2 || index + descriptor_length > configuration.len() {
                return Err("truncated xHCI configuration descriptor");
            }
            if configuration[index + 1] == 5 && descriptor_length >= 7 {
                endpoints.push(EndpointProfile {
                    address: configuration[index + 2],
                    transfer_type: configuration[index + 3] & 3,
                    max_packet: u16::from_le_bytes([
                        configuration[index + 4],
                        configuration[index + 5],
                    ]) & 0x7ff,
                });
            }
            index += descriptor_length;
        }
        crate::serial_println!(
            "[XHCI] descriptors fetched slot={} vid={:02x}{:02x} pid={:02x}{:02x} endpoints={}",
            slot,
            device[9],
            device[8],
            device[11],
            device[10],
            endpoints.len()
        );
        Ok((header[5], endpoints))
    }

    fn configure_endpoints(
        &mut self,
        slot: u8,
        port: u8,
        speed: u8,
        endpoints: &[EndpointProfile],
    ) -> Result<(), &'static str> {
        let offset = VirtAddr::new(self.physical_memory_offset);
        let input = (offset + self.input_context).as_mut_ptr::<u8>();
        let mut add_flags = 1u32;
        let mut highest_dci = 1u8;
        // SAFETY: the Address Device command has completed, so the dedicated
        // input context may be rebuilt for Configure Endpoint.
        unsafe { write_bytes(input, 0, 4096) };
        for endpoint in endpoints
            .iter()
            .filter(|endpoint| endpoint.transfer_type != 0)
        {
            let number = endpoint.address & 0x0f;
            let input_direction = endpoint.address & 0x80 != 0;
            let dci = number.saturating_mul(2) + u8::from(input_direction);
            if dci <= 1 || dci >= 32 || endpoint.max_packet == 0 {
                continue;
            }
            let ring = if input_direction {
                self.bulk_in_ring
            } else {
                self.bulk_out_ring
            };
            if input_direction && self.input_endpoint_dci == 0 {
                self.input_endpoint_dci = dci;
                self.input_max_packet = endpoint.max_packet;
                self.input_transfer_type = endpoint.transfer_type;
            }
            let endpoint_type = match (endpoint.transfer_type, input_direction) {
                (2, false) => 2,
                (2, true) => 6,
                (3, false) => 3,
                (3, true) => 7,
                _ => continue,
            };
            let context = usize::from(self.context_size) * (usize::from(dci) + 1);
            // SAFETY: DCI is validated below 32 and the context page holds all
            // 32 controller contexts at either supported context size.
            unsafe {
                write_volatile(
                    input.add(context + 4).cast::<u32>(),
                    (3 << 1) | (endpoint_type << 3) | (u32::from(endpoint.max_packet) << 16),
                );
                write_volatile(input.add(context + 8).cast::<u64>(), ring | 1);
                write_volatile(
                    input.add(context + 16).cast::<u32>(),
                    u32::from(endpoint.max_packet),
                );
            }
            add_flags |= 1 << dci;
            highest_dci = highest_dci.max(dci);
        }
        if highest_dci == 1 {
            return Err("no configurable xHCI endpoints found");
        }
        // SAFETY: input control and slot contexts occupy validated offsets.
        unsafe {
            write_volatile(input.add(4).cast::<u32>(), add_flags);
            let slot_context = input.add(usize::from(self.context_size));
            write_volatile(
                slot_context.cast::<u32>(),
                (u32::from(speed) << 20) | (u32::from(highest_dci) << 27),
            );
            write_volatile(slot_context.add(4).cast::<u32>(), u32::from(port) << 16);
        }
        self.submit_command(
            self.input_context,
            0,
            TRB_TYPE_CONFIGURE_ENDPOINT | (u32::from(slot) << 24),
        )?;
        Ok(())
    }

    fn poll_interrupt_input(&mut self) {
        if self.enabled_slot == 0
            || self.input_endpoint_dci == 0
            || self.input_transfer_type != 3
            || self.input_max_packet == 0
            || self.input_enqueue >= 255
        {
            return;
        }
        let offset = VirtAddr::new(self.physical_memory_offset);
        let ring = (offset + self.bulk_in_ring).as_mut_ptr::<Trb>();
        let data = (offset + self.transfer_data).as_mut_ptr::<u8>();
        let index = usize::from(self.input_enqueue);
        // SAFETY: the endpoint owns this transfer ring and data page; enqueue
        // is bounded before the reserved final TRB and IOC requests an event.
        unsafe {
            write_bytes(data, 0, 4096);
            let trb = ring.add(index);
            write_volatile(&mut (*trb).parameter, self.transfer_data);
            write_volatile(&mut (*trb).status, u32::from(self.input_max_packet));
            write_volatile(
                &mut (*trb).control,
                TRB_TYPE_NORMAL | (1 << 2) | (1 << 5) | TRB_CYCLE,
            );
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            write_volatile(
                self.doorbell_ptr(self.enabled_slot),
                u32::from(self.input_endpoint_dci),
            );
        }
        self.input_enqueue += 1;
        if self.wait_event(TRB_TYPE_TRANSFER_EVENT).is_err() {
            return;
        }
        let report_len = usize::from(self.input_max_packet).min(8);
        let mut report = [0u8; 8];
        // SAFETY: the successful Transfer Event completes DMA before this copy.
        unsafe { core::ptr::copy_nonoverlapping(data, report.as_mut_ptr(), report_len) };
        crate::drivers::usb_hid::process_keyboard_report(
            &crate::drivers::usb_hid::HidKeyboardReport::new(&report[..report_len]),
        );
        self.input_reports = self.input_reports.saturating_add(1);
        crate::serial_println!(
            "[XHCI-HID] input report slot={} endpoint={} bytes={} count={}",
            self.enabled_slot,
            self.input_endpoint_dci,
            report_len,
            self.input_reports
        );
    }

    fn prepare_address_context(&self, slot: u8, port: u8, speed: u8) -> Result<(), &'static str> {
        let max_packet = match speed {
            4 | 5 => 512u16,
            3 => 64,
            1 | 2 => 8,
            _ => return Err("unsupported xHCI port speed"),
        };
        let offset = VirtAddr::new(self.physical_memory_offset);
        let input = (offset + self.input_context).as_mut_ptr::<u8>();
        let output = (offset + self.output_context).as_mut_ptr::<u8>();
        let transfer_dequeue = self.transfer_ring | 1;
        // SAFETY: input/output context pages are dedicated, zeroed DMA pages;
        // offsets follow the controller-advertised 32/64-byte context size.
        unsafe {
            write_bytes(input, 0, 4096);
            write_bytes(output, 0, 4096);
            write_volatile(input.add(4).cast::<u32>(), 0b11);
            let slot_context = input.add(usize::from(self.context_size));
            write_volatile(
                slot_context.cast::<u32>(),
                (u32::from(speed) << 20) | (1 << 27),
            );
            write_volatile(slot_context.add(4).cast::<u32>(), u32::from(port) << 16);
            let endpoint = input.add(usize::from(self.context_size) * 2);
            write_volatile(
                endpoint.add(4).cast::<u32>(),
                (3 << 1) | (4 << 3) | (u32::from(max_packet) << 16),
            );
            write_volatile(endpoint.add(8).cast::<u64>(), transfer_dequeue);
            write_volatile(endpoint.add(16).cast::<u32>(), 8);
            let dcbaa = (offset + self.dcbaa).as_mut_ptr::<u64>();
            write_volatile(dcbaa.add(usize::from(slot)), self.output_context);
        }
        Ok(())
    }
}

pub fn poll_runtime() {
    for controller in CONTROLLERS.lock().iter_mut() {
        controller.poll_interrupt_input();
    }
}

fn alloc_frame(allocator: &mut impl FrameAllocator<Size4KiB>) -> Result<u64, &'static str> {
    allocator
        .allocate_frame()
        .map(|frame| frame.start_address().as_u64())
        .ok_or("xHCI DMA frame allocation failed")
}

pub fn write_to_buffer(out: &mut [u8]) -> usize {
    let mut len = 0;
    for byte in b"BDF       BAR0               VER  PORTS SLOTS CMD-RING          XFER-RING         EVENT-RING        READY\n" { put(out, &mut len, *byte); }
    for controller in CONTROLLERS.lock().iter() {
        hex(out, &mut len, u64::from(controller.pci.bus), 2);
        put(out, &mut len, b':');
        hex(out, &mut len, u64::from(controller.pci.slot), 2);
        put(out, &mut len, b'.');
        dec(out, &mut len, u64::from(controller.pci.function));
        put(out, &mut len, b' ');
        hex(out, &mut len, controller.bar0, 16);
        put(out, &mut len, b' ');
        hex(out, &mut len, u64::from(controller.version), 4);
        put(out, &mut len, b' ');
        dec(out, &mut len, u64::from(controller.port_count));
        put(out, &mut len, b' ');
        dec(out, &mut len, u64::from(controller.max_slots));
        put(out, &mut len, b' ');
        hex(out, &mut len, controller.command_ring, 16);
        put(out, &mut len, b' ');
        hex(out, &mut len, controller.transfer_ring, 16);
        put(out, &mut len, b' ');
        hex(out, &mut len, controller.event_ring, 16);
        put(out, &mut len, b' ');
        dec(out, &mut len, controller.rings_ready as u64);
        for byte in b" slot=" {
            put(out, &mut len, *byte);
        }
        dec(out, &mut len, u64::from(controller.enabled_slot));
        for byte in b" port=" {
            put(out, &mut len, *byte);
        }
        dec(out, &mut len, u64::from(controller.addressed_port));
        for byte in b" completions=" {
            put(out, &mut len, *byte);
        }
        dec(out, &mut len, u64::from(controller.command_completions));
        put(out, &mut len, b'\n');
    }
    len
}
fn put(out: &mut [u8], len: &mut usize, byte: u8) {
    if *len < out.len() {
        out[*len] = byte;
        *len += 1;
    }
}
fn hex(out: &mut [u8], len: &mut usize, value: u64, digits: usize) {
    for shift in (0..digits).rev() {
        let nibble = ((value >> (shift * 4)) & 15) as u8;
        put(
            out,
            len,
            if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            },
        );
    }
}
fn dec(out: &mut [u8], len: &mut usize, mut value: u64) {
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
        put(out, len, *byte);
    }
}
