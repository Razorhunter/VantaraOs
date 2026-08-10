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
const EHCI_PROG_IF: u8 = 0x20;
const EHCI_BAR_INDEX: u8 = 0;
const EHCI_MMIO_WINDOW: u64 = 0xffff_9300_0000_0000;
const EHCI_MMIO_STRIDE: u64 = 0x1000;
const EHCI_MMIO_SIZE: u64 = 0x1000;
const CAPLENGTH_OFFSET: u64 = 0x00;
const HCSPARAMS_OFFSET: u64 = 0x04;
const HCCPARAMS_OFFSET: u64 = 0x08;
const USBCMD_OFFSET: u64 = 0x00;
const USBSTS_OFFSET: u64 = 0x04;
const USBINTR_OFFSET: u64 = 0x08;
const CTRLDSSEGMENT_OFFSET: u64 = 0x10;
const ASYNCLISTADDR_OFFSET: u64 = 0x18;
const CONFIGFLAG_OFFSET: u64 = 0x40;
const PORTSC_OFFSET: u64 = 0x44;
const USBCMD_RUN: u32 = 1 << 0;
const USBCMD_RESET: u32 = 1 << 1;
const USBCMD_ASYNC_ENABLE: u32 = 1 << 5;
const USBSTS_HALTED: u32 = 1 << 12;
const USBSTS_ASYNC_STATUS: u32 = 1 << 15;
const PORTSC_CONNECTED: u32 = 1 << 0;
const PORTSC_ENABLED: u32 = 1 << 2;
const PORTSC_RESET: u32 = 1 << 8;
const PORTSC_OWNER: u32 = 1 << 13;
const EHCI_LINK_QH: u32 = 0b10;
const EHCI_LINK_TERMINATE: u32 = 1;
const QTD_TOKEN_HALTED: u32 = 1 << 6;
const QTD_TOKEN_ACTIVE: u32 = 1 << 7;
const QTD_TOKEN_IOC: u32 = 1 << 15;
const QTD_PID_OUT: u32 = 0 << 8;
const QTD_PID_IN: u32 = 1 << 8;
const QTD_PID_SETUP: u32 = 2 << 8;
const USB_REQUEST_GET_DESCRIPTOR: u8 = 6;
const USB_REQUEST_SET_ADDRESS: u8 = 5;
const USB_REQUEST_SET_CONFIGURATION: u8 = 9;
const WAIT_LIMIT: usize = 1_000_000;

#[repr(C)]
struct EhciQtd {
    next: u32,
    alternate_next: u32,
    token: u32,
    buffers: [u32; 5],
    extended_buffers: [u32; 5],
}

#[repr(C, align(32))]
struct EhciQueueHead {
    horizontal_link: u32,
    endpoint_characteristics: u32,
    endpoint_capabilities: u32,
    current_qtd: u32,
    overlay: EhciQtd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EhciController {
    pub pci: PciDevice,
    pub bar0: u64,
    pub mmio_ready: bool,
    pub mmio_base: u64,
    pub cap_length: u8,
    pub hci_version: u16,
    pub structural_params: u32,
    pub capability_params: u32,
    pub operational_command: u32,
    pub operational_status: u32,
    pub port_count: u8,
    pub companion_count: u8,
    pub routing_rules: bool,
    pub config_flag: u32,
    pub async_schedule: u32,
    pub async_ready: bool,
    pub companion_handoffs: u8,
    pub enumerated_devices: u8,
    pub last_vendor_id: u16,
    pub last_product_id: u16,
    pub bulk_inquiry_ok: bool,
    physical_memory_offset: u64,
    qtd_arena: u32,
    data_page: u32,
}

lazy_static! {
    static ref CONTROLLERS: Mutex<Vec<EhciController>> = Mutex::new(Vec::new());
}

pub fn init(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    physical_memory_offset: VirtAddr,
) {
    let devices: Vec<_> = pci::devices_by_class(USB_CLASS)
        .into_iter()
        .filter(|device| device.subclass == USB_SUBCLASS && device.prog_if == EHCI_PROG_IF)
        .collect();
    if devices.is_empty() {
        crate::drivers::status::report(
            "usb-ehci",
            crate::drivers::status::DriverState::Missing,
            "controller not found",
        );
        crate::serial_println!("[EHCI] no USB 2.0 controller detected");
        return;
    }

    let mut controllers = Vec::new();
    for (index, device) in devices.into_iter().enumerate() {
        let Some(bar0) = memory_bar(device) else {
            crate::serial_println!(
                "[EHCI] invalid BAR0 at {:02x}:{:02x}.{}",
                device.bus,
                device.slot,
                device.function
            );
            continue;
        };
        let Some(mmio_base) = EHCI_MMIO_WINDOW.checked_add(index as u64 * EHCI_MMIO_STRIDE) else {
            continue;
        };
        pci::enable_memory_and_bus_master(device);
        if crate::memory::map_mmio_range(
            PhysAddr::new(bar0),
            VirtAddr::new(mmio_base),
            EHCI_MMIO_SIZE,
            mapper,
            frame_allocator,
        )
        .is_err()
        {
            crate::serial_println!("[EHCI] MMIO map failed BAR0={:#x}", bar0);
            continue;
        }
        let mut controller = read_controller(device, bar0, mmio_base);
        controller.physical_memory_offset = physical_memory_offset.as_u64();
        match controller.initialize_async_schedule(frame_allocator) {
            Ok(()) => {
                crate::serial_println!(
                    "[EHCI] async schedule ready qh={:#x} qtd={:#x} data={:#x}",
                    controller.async_schedule,
                    controller.qtd_arena,
                    controller.data_page
                );
            }
            Err(error) => {
                crate::serial_println!("[EHCI] async schedule unavailable: {}", error);
            }
        }
        controller.companion_handoffs = controller.route_ports();
        controller.enumerate_high_speed_ports();
        crate::serial_println!(
            "[EHCI] {:02x}:{:02x}.{} bar0={:#x} caplen={} version={:04x} ports={} companions={} routing={} cmd={:#x} sts={:#x}",
            device.bus,
            device.slot,
            device.function,
            bar0,
            controller.cap_length,
            controller.hci_version,
            controller.port_count,
            controller.companion_count,
            controller.routing_rules,
            controller.operational_command,
            controller.operational_status,
        );
        crate::serial_println!(
            "[EHCI] routing configured flag={} companion-handoffs={}",
            controller.config_flag,
            controller.companion_handoffs
        );
        controllers.push(controller);
    }
    let state = if controllers.is_empty() {
        crate::drivers::status::DriverState::Error
    } else {
        crate::drivers::status::DriverState::Ready
    };
    crate::drivers::status::report("usb-ehci", state, "USB 2.0 capability registers mapped");
    *CONTROLLERS.lock() = controllers;
}

fn memory_bar(device: PciDevice) -> Option<u64> {
    match pci::read_bar_info(device, EHCI_BAR_INDEX) {
        PciBar::Memory32 { address, .. } | PciBar::Memory64 { address, .. } if address != 0 => {
            Some(address)
        }
        _ => None,
    }
}

fn read_controller(pci: PciDevice, bar0: u64, mmio_base: u64) -> EhciController {
    // SAFETY: the one-page uncached MMIO mapping is owned by this EHCI
    // controller; capability reads do not change controller state.
    unsafe {
        let cap = mmio_base as *const u8;
        let cap_length = read_volatile(cap.add(CAPLENGTH_OFFSET as usize));
        let version = read_volatile(cap.add(2).cast::<u16>());
        let structural_params = read_volatile(cap.add(HCSPARAMS_OFFSET as usize).cast::<u32>());
        let capability_params = read_volatile(cap.add(HCCPARAMS_OFFSET as usize).cast::<u32>());
        let operational = cap.add(usize::from(cap_length));
        let operational_command =
            read_volatile(operational.add(USBCMD_OFFSET as usize).cast::<u32>());
        let operational_status =
            read_volatile(operational.add(USBSTS_OFFSET as usize).cast::<u32>());
        let config_flag = if usize::from(cap_length) + CONFIGFLAG_OFFSET as usize + 4
            <= EHCI_MMIO_SIZE as usize
        {
            read_volatile(operational.add(CONFIGFLAG_OFFSET as usize).cast::<u32>())
        } else {
            0
        };
        EhciController {
            pci,
            bar0,
            mmio_ready: true,
            mmio_base,
            cap_length,
            hci_version: version,
            structural_params,
            capability_params,
            operational_command,
            operational_status,
            port_count: (structural_params & 0x0f) as u8,
            companion_count: ((structural_params >> 12) & 0x0f) as u8,
            routing_rules: capability_params & 1 != 0,
            config_flag,
            async_schedule: 0,
            async_ready: false,
            companion_handoffs: 0,
            enumerated_devices: 0,
            last_vendor_id: 0,
            last_product_id: 0,
            bulk_inquiry_ok: false,
            physical_memory_offset: 0,
            qtd_arena: 0,
            data_page: 0,
        }
    }
}

impl EhciController {
    fn operational_ptr(&self, offset: u64) -> *mut u32 {
        (self.mmio_base + u64::from(self.cap_length) + offset) as *mut u32
    }

    fn read_op(&self, offset: u64) -> u32 {
        // SAFETY: init owns the mapped EHCI operational-register window.
        unsafe { read_volatile(self.operational_ptr(offset)) }
    }

    fn write_op(&self, offset: u64, value: u32) {
        // SAFETY: init owns the mapped EHCI operational-register window.
        unsafe { write_volatile(self.operational_ptr(offset), value) }
    }

    fn wait_op(&self, offset: u64, mask: u32, expected: u32) -> bool {
        for _ in 0..WAIT_LIMIT {
            if self.read_op(offset) & mask == expected {
                return true;
            }
            core::hint::spin_loop();
        }
        false
    }

    fn initialize_async_schedule(
        &mut self,
        frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    ) -> Result<(), &'static str> {
        self.take_ownership_from_firmware()?;
        let qh = allocate_dma32_frame(frame_allocator)?;
        let qtd = allocate_dma32_frame(frame_allocator)?;
        let data = allocate_dma32_frame(frame_allocator)?;
        let offset = VirtAddr::new(self.physical_memory_offset);
        // SAFETY: these are three distinct, exclusively allocated pages in the
        // bootloader's physical-memory mapping.
        unsafe {
            write_bytes((offset + u64::from(qh)).as_mut_ptr::<u8>(), 0, 4096);
            write_bytes((offset + u64::from(qtd)).as_mut_ptr::<u8>(), 0, 4096);
            write_bytes((offset + u64::from(data)).as_mut_ptr::<u8>(), 0, 4096);
            let head = (offset + u64::from(qh)).as_mut_ptr::<EhciQueueHead>();
            write_volatile(&mut (*head).horizontal_link, qh | EHCI_LINK_QH);
            // H=1, high-speed endpoint, max packet 64. The reclamation head is
            // deliberately idle until submit_qtd installs work in its overlay.
            write_volatile(
                &mut (*head).endpoint_characteristics,
                (1 << 15) | (2 << 12) | (64 << 16),
            );
            write_volatile(&mut (*head).overlay.next, EHCI_LINK_TERMINATE);
            write_volatile(&mut (*head).overlay.alternate_next, EHCI_LINK_TERMINATE);
        }

        self.write_op(USBINTR_OFFSET, 0);
        self.write_op(USBCMD_OFFSET, self.read_op(USBCMD_OFFSET) & !USBCMD_RUN);
        if !self.wait_op(USBSTS_OFFSET, USBSTS_HALTED, USBSTS_HALTED) {
            return Err("controller did not halt");
        }
        self.write_op(USBCMD_OFFSET, USBCMD_RESET);
        if !self.wait_op(USBCMD_OFFSET, USBCMD_RESET, 0) {
            return Err("host-controller reset timed out");
        }
        self.write_op(CTRLDSSEGMENT_OFFSET, 0);
        self.write_op(ASYNCLISTADDR_OFFSET, qh);
        self.write_op(CONFIGFLAG_OFFSET, 1);
        self.write_op(USBCMD_OFFSET, USBCMD_RUN | USBCMD_ASYNC_ENABLE | (8 << 16));
        if !self.wait_op(USBSTS_OFFSET, USBSTS_HALTED, 0)
            || !self.wait_op(USBSTS_OFFSET, USBSTS_ASYNC_STATUS, USBSTS_ASYNC_STATUS)
        {
            return Err("asynchronous schedule did not start");
        }
        self.async_schedule = qh;
        self.qtd_arena = qtd;
        self.data_page = data;
        self.async_ready = true;
        self.operational_command = self.read_op(USBCMD_OFFSET);
        self.operational_status = self.read_op(USBSTS_OFFSET);
        self.config_flag = self.read_op(CONFIGFLAG_OFFSET);
        Ok(())
    }

    fn take_ownership_from_firmware(&self) -> Result<(), &'static str> {
        let eecp = ((self.capability_params >> 8) & 0xff) as u8;
        if eecp < 0x40 {
            return Ok(());
        }
        let legacy = pci::read_config_u32(self.pci.bus, self.pci.slot, self.pci.function, eecp);
        if legacy & (1 << 16) == 0 {
            return Ok(());
        }
        pci::write_config_u32(
            self.pci.bus,
            self.pci.slot,
            self.pci.function,
            eecp,
            legacy | (1 << 24),
        );
        for _ in 0..WAIT_LIMIT {
            let value = pci::read_config_u32(self.pci.bus, self.pci.slot, self.pci.function, eecp);
            if value & (1 << 16) == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err("firmware ownership handoff timed out")
    }

    fn route_ports(&self) -> u8 {
        let mut handed_off = 0u8;
        for port in 0..self.port_count {
            let register = PORTSC_OFFSET + u64::from(port) * 4;
            let status = self.read_op(register);
            if status & PORTSC_CONNECTED == 0 {
                continue;
            }
            self.write_op(register, (status & !(0x2a | PORTSC_OWNER)) | PORTSC_RESET);
            for _ in 0..100_000 {
                core::hint::spin_loop();
            }
            self.write_op(register, self.read_op(register) & !PORTSC_RESET);
            if !self.wait_op(register, PORTSC_RESET, 0) {
                continue;
            }
            let after_reset = self.read_op(register);
            if after_reset & PORTSC_ENABLED == 0 && self.companion_count != 0 {
                self.write_op(register, (after_reset & !0x2a) | PORTSC_OWNER);
                handed_off = handed_off.saturating_add(1);
            }
        }
        handed_off
    }

    /// Submit one bounded high-speed qTD through the asynchronous reclamation
    /// head. Control transfers use this primitive for their SETUP, DATA and
    /// STATUS stages; bulk endpoints use IN/OUT directly.
    fn submit_qtd(
        &self,
        device_address: u8,
        endpoint: u8,
        max_packet: u16,
        pid: u8,
        data_toggle: bool,
        bytes: &mut [u8],
    ) -> Result<usize, &'static str> {
        if !self.async_ready || bytes.len() > 4096 || device_address > 127 || endpoint > 15 {
            return Err("invalid EHCI qTD request");
        }
        let pid_bits = match pid {
            0 => QTD_PID_OUT,
            1 => QTD_PID_IN,
            2 => QTD_PID_SETUP,
            _ => return Err("invalid EHCI qTD PID"),
        };
        let offset = VirtAddr::new(self.physical_memory_offset);
        let head = (offset + u64::from(self.async_schedule)).as_mut_ptr::<EhciQueueHead>();
        let qtd = (offset + u64::from(self.qtd_arena)).as_mut_ptr::<EhciQtd>();
        let token = QTD_TOKEN_ACTIVE
            | QTD_TOKEN_IOC
            | pid_bits
            | (3 << 10)
            | ((data_toggle as u32) << 31)
            | ((bytes.len() as u32) << 16);
        let data = (offset + u64::from(self.data_page)).as_mut_ptr::<u8>();
        self.write_op(
            USBCMD_OFFSET,
            self.read_op(USBCMD_OFFSET) & !USBCMD_ASYNC_ENABLE,
        );
        if !self.wait_op(USBSTS_OFFSET, USBSTS_ASYNC_STATUS, 0) {
            return Err("EHCI asynchronous schedule did not stop");
        }
        // SAFETY: the schedule owns these DMA pages for the controller's
        // lifetime. Volatile stores publish a completely initialized qTD last.
        unsafe {
            write_bytes(qtd.cast::<u8>(), 0, core::mem::size_of::<EhciQtd>());
            if matches!(pid, 0 | 2) && !bytes.is_empty() {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
            }
            write_volatile(&mut (*qtd).next, EHCI_LINK_TERMINATE);
            write_volatile(&mut (*qtd).alternate_next, EHCI_LINK_TERMINATE);
            write_volatile(&mut (*qtd).buffers[0], self.data_page);
            write_volatile(&mut (*qtd).token, token);
            write_volatile(
                &mut (*head).endpoint_characteristics,
                u32::from(device_address)
                    | (u32::from(endpoint) << 8)
                    | (2 << 12)
                    | (1 << 14)
                    | (1 << 15)
                    | (u32::from(max_packet) << 16),
            );
            write_volatile(&mut (*head).current_qtd, 0);
            write_volatile(&mut (*head).overlay.token, 0);
            write_volatile(&mut (*head).overlay.next, self.qtd_arena);
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.write_op(
            USBCMD_OFFSET,
            self.read_op(USBCMD_OFFSET) | USBCMD_ASYNC_ENABLE,
        );
        if !self.wait_op(USBSTS_OFFSET, USBSTS_ASYNC_STATUS, USBSTS_ASYNC_STATUS) {
            return Err("EHCI asynchronous schedule did not restart");
        }
        for _ in 0..WAIT_LIMIT {
            // SAFETY: token is controller-owned DMA state and must be polled
            // with volatile reads until Active clears.
            let completion = unsafe { read_volatile(&(*qtd).token) };
            if completion & QTD_TOKEN_ACTIVE == 0 {
                // SAFETY: qTD is no longer active and can be detached.
                unsafe { write_volatile(&mut (*head).overlay.next, EHCI_LINK_TERMINATE) };
                if completion & QTD_TOKEN_HALTED != 0 || completion & 0x7c != 0 {
                    return Err("EHCI qTD completed with error");
                }
                let remaining = ((completion >> 16) & 0x7fff) as usize;
                let transferred = bytes.len().saturating_sub(remaining);
                if pid == 1 && transferred != 0 {
                    // SAFETY: the controller completed the IN qTD into the
                    // dedicated data page and `transferred` is buffer-bounded.
                    unsafe {
                        core::ptr::copy_nonoverlapping(data, bytes.as_mut_ptr(), transferred)
                    };
                }
                return Ok(transferred);
            }
            core::hint::spin_loop();
        }
        Err("EHCI qTD timed out")
    }

    fn control_transfer(
        &self,
        address: u8,
        max_packet: u16,
        setup: [u8; 8],
        direction_in: Option<bool>,
        data: &mut [u8],
    ) -> Result<usize, &'static str> {
        let mut setup_packet = setup;
        self.submit_qtd(address, 0, max_packet, 2, false, &mut setup_packet)?;
        let transferred = if let Some(direction_in) = direction_in {
            self.submit_qtd(address, 0, max_packet, u8::from(direction_in), true, data)?
        } else {
            0
        };
        let status_pid = if direction_in == Some(true) { 0 } else { 1 };
        self.submit_qtd(address, 0, max_packet, status_pid, true, &mut [])?;
        Ok(transferred)
    }

    fn enumerate_high_speed_ports(&mut self) {
        let mut address = 1u8;
        for port in 0..self.port_count {
            let status = self.read_op(PORTSC_OFFSET + u64::from(port) * 4);
            if status & (PORTSC_CONNECTED | PORTSC_ENABLED) != (PORTSC_CONNECTED | PORTSC_ENABLED)
                || status & PORTSC_OWNER != 0
            {
                continue;
            }
            match self.enumerate_device(address) {
                Ok((vendor, product)) => {
                    self.enumerated_devices = self.enumerated_devices.saturating_add(1);
                    self.last_vendor_id = vendor;
                    self.last_product_id = product;
                    crate::serial_println!(
                        "[EHCI] enumerated high-speed port={} addr={} vid={:04x} pid={:04x}",
                        port,
                        address,
                        vendor,
                        product
                    );
                    address = address.saturating_add(1);
                }
                Err(error) => {
                    crate::serial_println!("[EHCI] enumeration failed port={}: {}", port, error);
                }
            }
        }
    }

    fn enumerate_device(&mut self, address: u8) -> Result<(u16, u16), &'static str> {
        let mut first = [0u8; 8];
        self.control_transfer(
            0,
            64,
            control_setup(0x80, USB_REQUEST_GET_DESCRIPTOR, 1 << 8, 0, 8),
            Some(true),
            &mut first,
        )?;
        if first[0] != 18 || first[1] != 1 || first[7] != 64 {
            return Err("invalid high-speed device descriptor");
        }
        self.control_transfer(
            0,
            64,
            control_setup(0, USB_REQUEST_SET_ADDRESS, u16::from(address), 0, 0),
            None,
            &mut [],
        )?;
        for _ in 0..100_000 {
            core::hint::spin_loop();
        }
        let mut descriptor = [0u8; 18];
        self.control_transfer(
            address,
            64,
            control_setup(0x80, USB_REQUEST_GET_DESCRIPTOR, 1 << 8, 0, 18),
            Some(true),
            &mut descriptor,
        )?;
        let vendor = u16::from_le_bytes([descriptor[8], descriptor[9]]);
        let product = u16::from_le_bytes([descriptor[10], descriptor[11]]);
        let mut configuration = [0u8; 9];
        self.control_transfer(
            address,
            64,
            control_setup(0x80, USB_REQUEST_GET_DESCRIPTOR, 2 << 8, 0, 9),
            Some(true),
            &mut configuration,
        )?;
        if configuration[0] != 9 || configuration[1] != 2 || configuration[5] == 0 {
            return Err("invalid configuration descriptor");
        }
        let total = usize::from(u16::from_le_bytes([configuration[2], configuration[3]]));
        if !(9..=4096).contains(&total) {
            return Err("invalid EHCI configuration length");
        }
        let mut descriptors = alloc::vec![0u8; total];
        self.control_transfer(
            address,
            64,
            control_setup(0x80, USB_REQUEST_GET_DESCRIPTOR, 2 << 8, 0, total as u16),
            Some(true),
            &mut descriptors,
        )?;
        self.control_transfer(
            address,
            64,
            control_setup(
                0,
                USB_REQUEST_SET_CONFIGURATION,
                u16::from(configuration[5]),
                0,
                0,
            ),
            None,
            &mut [],
        )?;
        if let Some((bulk_out, out_packet, bulk_in, in_packet)) =
            find_mass_storage_endpoints(&descriptors)
        {
            self.bulk_inquiry_ok = self
                .bulk_only_inquiry(address, bulk_out, out_packet, bulk_in, in_packet)
                .is_ok();
            if self.bulk_inquiry_ok {
                crate::serial_println!(
                    "[EHCI-BULK] SCSI INQUIRY complete addr={} bulk-out=0x{:02x} bulk-in=0x{:02x}",
                    address,
                    bulk_out,
                    bulk_in
                );
            }
        }
        Ok((vendor, product))
    }

    fn bulk_only_inquiry(
        &self,
        address: u8,
        bulk_out: u8,
        out_packet: u16,
        bulk_in: u8,
        in_packet: u16,
    ) -> Result<(), &'static str> {
        let mut cbw = [0u8; 31];
        cbw[0..4].copy_from_slice(&0x4342_5355u32.to_le_bytes());
        cbw[4..8].copy_from_slice(&1u32.to_le_bytes());
        cbw[8..12].copy_from_slice(&36u32.to_le_bytes());
        cbw[12] = 0x80;
        cbw[14] = 6;
        cbw[15] = 0x12;
        cbw[19] = 36;
        self.submit_qtd(address, bulk_out & 0x0f, out_packet, 0, false, &mut cbw)?;
        let mut inquiry = [0u8; 36];
        self.submit_qtd(address, bulk_in & 0x0f, in_packet, 1, false, &mut inquiry)?;
        let mut csw = [0u8; 13];
        self.submit_qtd(address, bulk_in & 0x0f, in_packet, 1, true, &mut csw)?;
        if u32::from_le_bytes([csw[0], csw[1], csw[2], csw[3]]) != 0x5342_5355
            || u32::from_le_bytes([csw[4], csw[5], csw[6], csw[7]]) != 1
            || csw[12] != 0
        {
            return Err("invalid EHCI Bulk-Only CSW");
        }
        Ok(())
    }
}

fn find_mass_storage_endpoints(bytes: &[u8]) -> Option<(u8, u16, u8, u16)> {
    let mut index = 0usize;
    let mut mass_storage = false;
    let mut bulk_out = None;
    let mut bulk_in = None;
    while index + 2 <= bytes.len() {
        let length = usize::from(bytes[index]);
        if length < 2 || index + length > bytes.len() {
            return None;
        }
        match bytes[index + 1] {
            4 if length >= 9 => {
                mass_storage = bytes[index + 5..index + 8] == [0x08, 0x06, 0x50];
                bulk_out = None;
                bulk_in = None;
            }
            5 if length >= 7 && mass_storage && bytes[index + 3] & 3 == 2 => {
                let address = bytes[index + 2];
                let packet = u16::from_le_bytes([bytes[index + 4], bytes[index + 5]]) & 0x7ff;
                if address & 0x80 != 0 {
                    bulk_in = Some((address, packet));
                } else {
                    bulk_out = Some((address, packet));
                }
                if let (Some((out, out_packet)), Some((input, in_packet))) = (bulk_out, bulk_in) {
                    return Some((out, out_packet, input, in_packet));
                }
            }
            _ => {}
        }
        index += length;
    }
    None
}

fn control_setup(request_type: u8, request: u8, value: u16, index: u16, length: u16) -> [u8; 8] {
    let value = value.to_le_bytes();
    let index = index.to_le_bytes();
    let length = length.to_le_bytes();
    [
        request_type,
        request,
        value[0],
        value[1],
        index[0],
        index[1],
        length[0],
        length[1],
    ]
}

fn allocate_dma32_frame(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<u32, &'static str> {
    let physical = frame_allocator
        .allocate_frame()
        .ok_or("EHCI DMA frame allocation failed")?
        .start_address()
        .as_u64();
    u32::try_from(physical).map_err(|_| "EHCI requires DMA frames below 4 GiB")
}

pub fn write_to_buffer(out: &mut [u8]) -> usize {
    let controllers = CONTROLLERS.lock();
    let mut len = 0usize;
    for byte in b"BDF       BAR0               VER  PORTS COMP ROUTING CMD        STS        CFG\n"
    {
        if len < out.len() {
            out[len] = *byte;
            len += 1;
        }
    }
    for controller in controllers.iter() {
        len = append_hex(out, len, u64::from(controller.pci.bus), 2);
        len = append_byte(out, len, b':');
        len = append_hex(out, len, u64::from(controller.pci.slot), 2);
        len = append_byte(out, len, b'.');
        len = append_dec(out, len, u64::from(controller.pci.function));
        len = append_str(out, len, b"  0x");
        len = append_hex(out, len, controller.bar0, 16);
        len = append_byte(out, len, b' ');
        len = append_hex(out, len, u64::from(controller.hci_version), 4);
        len = append_byte(out, len, b' ');
        len = append_dec(out, len, u64::from(controller.port_count));
        len = append_byte(out, len, b' ');
        len = append_dec(out, len, u64::from(controller.companion_count));
        len = append_byte(out, len, b' ');
        len = append_dec(out, len, controller.routing_rules as u64);
        len = append_byte(out, len, b' ');
        len = append_hex(out, len, u64::from(controller.operational_command), 8);
        len = append_byte(out, len, b' ');
        len = append_hex(out, len, u64::from(controller.operational_status), 8);
        len = append_byte(out, len, b' ');
        len = append_hex(out, len, u64::from(controller.config_flag), 8);
        len = append_str(out, len, b" async=0x");
        len = append_hex(out, len, u64::from(controller.async_schedule), 8);
        len = append_str(out, len, b" handoff=");
        len = append_dec(out, len, u64::from(controller.companion_handoffs));
        len = append_str(out, len, b" devices=");
        len = append_dec(out, len, u64::from(controller.enumerated_devices));
        len = append_str(out, len, b" last=");
        len = append_hex(out, len, u64::from(controller.last_vendor_id), 4);
        len = append_byte(out, len, b':');
        len = append_hex(out, len, u64::from(controller.last_product_id), 4);
        len = append_str(out, len, b" bulk-inquiry=");
        len = append_dec(out, len, controller.bulk_inquiry_ok as u64);
        len = append_byte(out, len, b'\n');
    }
    len
}

fn append_byte(out: &mut [u8], len: usize, byte: u8) -> usize {
    if len < out.len() {
        out[len] = byte;
        len + 1
    } else {
        len
    }
}
fn append_str(out: &mut [u8], mut len: usize, text: &[u8]) -> usize {
    for byte in text {
        len = append_byte(out, len, *byte);
    }
    len
}
fn append_hex(out: &mut [u8], mut len: usize, value: u64, digits: usize) -> usize {
    for shift in (0..digits).rev() {
        let nibble = ((value >> (shift * 4)) & 15) as u8;
        len = append_byte(
            out,
            len,
            if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            },
        );
    }
    len
}
fn append_dec(out: &mut [u8], mut len: usize, mut value: u64) -> usize {
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
        len = append_byte(out, len, *byte);
    }
    len
}

#[cfg(test)]
mod tests {
    use super::{EhciQtd, EhciQueueHead, control_setup};

    #[test_case]
    fn dma_layout_matches_ehci_overlay_offsets() {
        assert_eq!(core::mem::offset_of!(EhciQueueHead, overlay), 16);
        assert_eq!(core::mem::offset_of!(EhciQtd, token), 8);
        assert_eq!(core::mem::offset_of!(EhciQtd, buffers), 12);
    }

    #[test_case]
    fn encodes_standard_control_request() {
        assert_eq!(
            control_setup(0x80, 6, 0x0100, 0, 18),
            [0x80, 6, 0, 1, 0, 0, 18, 0]
        );
    }
}
