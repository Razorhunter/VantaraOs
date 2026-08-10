// UHCI (Universal Host Controller Interface) implementation
use crate::drivers::pci;
use crate::drivers::usb_bulk_only::BulkOnlyTransport;
use crate::drivers::usb_bulk_only::UsbBulkPipe;
use crate::drivers::usb_descriptor::{
    DescriptorIter, ParsedDescriptor, UsbDescriptorError, parse_device_descriptor,
};
use crate::drivers::usb_hid::{
    HidDeviceType, HidKeyboardReport, HidMouseReport, process_keyboard_report, process_mouse_report,
};
use crate::drivers::usb_mass_storage::{
    UsbMassStorageDetectionError, UsbMassStorageInterface, detect_interface,
};
#[cfg(feature = "usb-write-test")]
use crate::drivers::usb_scsi::UsbMassStorageWritableBlockDevice;
use crate::drivers::usb_scsi::{ScsiError, UsbScsiDevice};
use crate::drivers::usb_structs::*;
#[cfg(feature = "usb-write-test")]
use crate::storage::block::{BLOCK_SIZE, BlockDevice};
use crate::sync::PreemptMutex as Mutex;
use alloc::vec::Vec;
use lazy_static::lazy_static;
use x86_64::VirtAddr;
use x86_64::instructions::port::Port;
use x86_64::structures::paging::{FrameAllocator, Size4KiB};

pub const UHCI_MAX_DEVICES: usize = 127;
const UHCI_FRAME_COUNT: usize = 1024;
const UHCI_LINK_TERMINATE: u32 = 1;
const UHCI_LINK_QUEUE_HEAD: u32 = 1 << 1;
const UHCI_LINK_DEPTH_FIRST: u32 = 1 << 2;
const UHCI_FRAME_LIST_BASE: u16 = 0x08;
const UHCI_FRAME_NUMBER: u16 = 0x06;
const UHCI_INTERRUPT_ENABLE: u16 = 0x04;
const UHCI_STATUS_REGISTER: u16 = 0x02;
const UHCI_TD_ACTIVE: u32 = 1 << 23;
const UHCI_TD_ERROR_MASK: u32 = 0x3f << 17;
const UHCI_TD_ERROR_RETRIES: u32 = 3 << 27;
const UHCI_MAX_TDS: usize = 4096 / core::mem::size_of::<UhciTransferDescriptor>();
const UHCI_TRANSFER_TIMEOUT_SPINS: usize = 5_000_000;
const UHCI_INTERRUPT_POLL_SPINS: usize = 20_000;
const USB_PID_IN: u8 = 0x69;
const USB_PID_OUT: u8 = 0xe1;
const USB_PID_SETUP: u8 = 0x2d;
const UHCI_PORT_CONNECTED: u16 = 1 << 0;
const UHCI_PORT_CONNECT_CHANGED: u16 = 1 << 1;
const UHCI_PORT_ENABLED: u16 = 1 << 2;
const UHCI_PORT_ENABLE_CHANGED: u16 = 1 << 3;
const UHCI_PORT_RESET: u16 = 1 << 9;
const USB_DESCRIPTOR_DEVICE: u16 = 1;
const USB_DESCRIPTOR_CONFIGURATION: u16 = 2;
const USB_REQUEST_GET_DESCRIPTOR: u8 = 6;
const USB_REQUEST_SET_ADDRESS: u8 = 5;
const USB_REQUEST_SET_CONFIGURATION: u8 = 9;
const USB_REQUEST_CLEAR_FEATURE: u8 = 1;
const USB_MASS_STORAGE_RESET: u8 = 0xff;
const USB_CONTROL_BUFFER_OFFSET: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UhciTransferError {
    ScheduleUnavailable,
    InvalidAddress,
    InvalidEndpoint,
    InvalidPacketSize,
    TransferTooLarge,
    Timeout,
    ControllerFault,
    TransactionFault { status: u32 },
    ShortPacket,
    DeviceDisconnected,
    DeviceEjected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UhciTransferDirection {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbEnumerationError {
    PortReset,
    Transfer(UhciTransferError),
    InvalidDescriptor,
    Descriptor(UsbDescriptorError),
}

impl From<UhciTransferError> for UsbEnumerationError {
    fn from(error: UhciTransferError) -> Self {
        Self::Transfer(error)
    }
}

impl From<UsbDescriptorError> for UsbEnumerationError {
    fn from(error: UsbDescriptorError) -> Self {
        Self::Descriptor(error)
    }
}

#[derive(Debug, Clone, Copy)]
struct UhciSchedule {
    frame_list: u32,
    queue_head: u32,
    transfer_descriptors: u32,
    data_buffer: u32,
    physical_memory_offset: u64,
}

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
    mass_storage_devices: Vec<UsbMassStorageDevice>,
    hid_devices: Vec<UsbHidRuntimeDevice>,
    schedule: Option<UhciSchedule>,
    last_port_poll_tick: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbHidRuntimeDevice {
    pub port: u8,
    pub address: u8,
    pub interface_number: u8,
    pub hid_type: HidDeviceType,
    pub interrupt_endpoint: u8,
    pub max_packet_size: u16,
    pub interval: u8,
    pub data_toggle: bool,
    pub last_poll_tick: u64,
    pub state: UsbDeviceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HidInterfaceProfile {
    interface_number: u8,
    hid_type: HidDeviceType,
    interrupt_endpoint: u8,
    max_packet_size: u16,
    interval: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbMassStorageDevice {
    pub port: u8,
    pub address: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub interface: UsbMassStorageInterface,
    pub control_max_packet_size: u8,
    pub ready: bool,
    pub block_count: u64,
    pub block_size: u32,
    pub lba0_checksum: u32,
    pub state: UsbDeviceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbDeviceState {
    Detected,
    Ready,
    Disconnected,
    Ejected,
}

struct UhciControllerPipe<'a> {
    controller: &'a mut UhciController,
    address: u8,
    interface_number: u8,
    control_max_packet_size: u8,
    bulk_in_endpoint: u8,
    bulk_in_max_packet_size: u16,
    bulk_out_endpoint: u8,
    bulk_out_max_packet_size: u16,
    in_toggle: bool,
    out_toggle: bool,
}

impl UsbBulkPipe for UhciControllerPipe<'_> {
    type Error = UhciTransferError;

    fn bulk_out(&mut self, endpoint: u8, bytes: &[u8]) -> Result<usize, Self::Error> {
        if endpoint != self.bulk_out_endpoint {
            return Err(UhciTransferError::InvalidEndpoint);
        }
        self.controller.ensure_device_active(self.address)?;
        let mut dma_bytes = bytes.to_vec();
        self.controller.bulk_transfer(
            self.address,
            endpoint & 0x0f,
            self.bulk_out_max_packet_size,
            UhciTransferDirection::Out,
            &mut self.out_toggle,
            &mut dma_bytes,
        )
    }

    fn bulk_in(&mut self, endpoint: u8, bytes: &mut [u8]) -> Result<usize, Self::Error> {
        if endpoint != self.bulk_in_endpoint {
            return Err(UhciTransferError::InvalidEndpoint);
        }
        self.controller.ensure_device_active(self.address)?;
        self.controller.bulk_transfer(
            self.address,
            endpoint & 0x0f,
            self.bulk_in_max_packet_size,
            UhciTransferDirection::In,
            &mut self.in_toggle,
            bytes,
        )
    }

    fn reset_recovery(&mut self) -> Result<(), Self::Error> {
        reset_mass_storage_endpoints(
            self.controller,
            self.address,
            self.control_max_packet_size,
            self.interface_number,
            self.bulk_in_endpoint,
            self.bulk_out_endpoint,
        )?;
        self.in_toggle = false;
        self.out_toggle = false;
        Ok(())
    }
}

pub struct UhciMassStoragePipe {
    address: u8,
    interface_number: u8,
    control_max_packet_size: u8,
    bulk_in_endpoint: u8,
    bulk_in_max_packet_size: u16,
    bulk_out_endpoint: u8,
    bulk_out_max_packet_size: u16,
    in_toggle: bool,
    out_toggle: bool,
}

impl UsbBulkPipe for UhciMassStoragePipe {
    type Error = UhciTransferError;

    fn bulk_out(&mut self, endpoint: u8, bytes: &[u8]) -> Result<usize, Self::Error> {
        if endpoint != self.bulk_out_endpoint {
            return Err(UhciTransferError::InvalidEndpoint);
        }
        let mut dma_bytes = bytes.to_vec();
        let mut controller = UHCI_CONTROLLER.lock();
        let controller = controller
            .as_mut()
            .ok_or(UhciTransferError::ScheduleUnavailable)?;
        controller.ensure_device_active(self.address)?;
        controller.bulk_transfer(
            self.address,
            endpoint & 0x0f,
            self.bulk_out_max_packet_size,
            UhciTransferDirection::Out,
            &mut self.out_toggle,
            &mut dma_bytes,
        )
    }

    fn bulk_in(&mut self, endpoint: u8, bytes: &mut [u8]) -> Result<usize, Self::Error> {
        if endpoint != self.bulk_in_endpoint {
            return Err(UhciTransferError::InvalidEndpoint);
        }
        let mut controller = UHCI_CONTROLLER.lock();
        let controller = controller
            .as_mut()
            .ok_or(UhciTransferError::ScheduleUnavailable)?;
        controller.ensure_device_active(self.address)?;
        controller.bulk_transfer(
            self.address,
            endpoint & 0x0f,
            self.bulk_in_max_packet_size,
            UhciTransferDirection::In,
            &mut self.in_toggle,
            bytes,
        )
    }

    fn reset_recovery(&mut self) -> Result<(), Self::Error> {
        let mut controller = UHCI_CONTROLLER.lock();
        reset_mass_storage_endpoints(
            controller
                .as_mut()
                .ok_or(UhciTransferError::ScheduleUnavailable)?,
            self.address,
            self.control_max_packet_size,
            self.interface_number,
            self.bulk_in_endpoint,
            self.bulk_out_endpoint,
        )?;
        self.in_toggle = false;
        self.out_toggle = false;
        Ok(())
    }
}

impl UhciMassStoragePipe {
    pub fn close(self) {}
}

impl UhciController {
    pub fn new(iobase: u16) -> Self {
        UhciController {
            iobase,
            devices: Vec::new(),
            mass_storage_devices: Vec::new(),
            hid_devices: Vec::new(),
            schedule: None,
            last_port_poll_tick: 0,
        }
    }

    pub fn init(&mut self) -> Result<(), &'static str> {
        crate::serial_println!(
            "[USB] Initializing UHCI controller at I/O base 0x{:x}",
            self.iobase
        );

        // Reset the controller
        self.reset()?;

        crate::serial_println!("[USB] UHCI controller reset successfully");
        Ok(())
    }

    pub fn initialize_schedule(
        &mut self,
        frame_allocator: &mut impl FrameAllocator<Size4KiB>,
        physical_memory_offset: VirtAddr,
    ) -> Result<(), &'static str> {
        let frame_list = allocate_uhci_frame(frame_allocator)?;
        let queue_head = allocate_uhci_frame(frame_allocator)?;
        let transfer_descriptors = allocate_uhci_frame(frame_allocator)?;
        let data_buffer = allocate_uhci_frame(frame_allocator)?;

        // SAFETY: each address is a distinct allocator-owned 4 KiB frame and
        // the bootloader physical-memory mapping covers all four frames.
        unsafe {
            core::ptr::write_bytes(
                (physical_memory_offset + u64::from(frame_list)).as_mut_ptr::<u8>(),
                0,
                4096,
            );
            core::ptr::write_bytes(
                (physical_memory_offset + u64::from(queue_head)).as_mut_ptr::<u8>(),
                0,
                4096,
            );
            core::ptr::write_bytes(
                (physical_memory_offset + u64::from(transfer_descriptors)).as_mut_ptr::<u8>(),
                0,
                4096,
            );
            core::ptr::write_bytes(
                (physical_memory_offset + u64::from(data_buffer)).as_mut_ptr::<u8>(),
                0,
                4096,
            );

            let frames = (physical_memory_offset + u64::from(frame_list)).as_mut_ptr::<u32>();
            for index in 0..UHCI_FRAME_COUNT {
                frames
                    .add(index)
                    .write_volatile(queue_head | UHCI_LINK_QUEUE_HEAD);
            }
            let qh = (physical_memory_offset + u64::from(queue_head)).as_mut_ptr::<UhciQueueHead>();
            qh.write_volatile(UhciQueueHead {
                link_pointer: UHCI_LINK_TERMINATE,
                element_link_pointer: UHCI_LINK_TERMINATE,
            });
        }

        let mut interrupt_enable = Port::<u16>::new(self.iobase + UHCI_INTERRUPT_ENABLE);
        let mut frame_number = Port::<u16>::new(self.iobase + UHCI_FRAME_NUMBER);
        let mut frame_base = Port::<u32>::new(self.iobase + UHCI_FRAME_LIST_BASE);
        // SAFETY: these ports belong to the discovered UHCI controller. The
        // controller is halted after reset while its valid schedule is set.
        unsafe {
            interrupt_enable.write(0);
            frame_number.write(0);
            frame_base.write(frame_list);
        }
        self.schedule = Some(UhciSchedule {
            frame_list,
            queue_head,
            transfer_descriptors,
            data_buffer,
            physical_memory_offset: physical_memory_offset.as_u64(),
        });
        crate::serial_println!(
            "[USB] UHCI schedule ready frame-list={:#x} qh={:#x} td={:#x} data={:#x}",
            frame_list,
            queue_head,
            transfer_descriptors,
            data_buffer
        );
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

    pub fn bulk_transfer(
        &mut self,
        address: u8,
        endpoint: u8,
        max_packet_size: u16,
        direction: UhciTransferDirection,
        data_toggle: &mut bool,
        bytes: &mut [u8],
    ) -> Result<usize, UhciTransferError> {
        self.transfer_with_options(
            address,
            endpoint,
            max_packet_size,
            direction,
            data_toggle,
            bytes,
            UHCI_TRANSFER_TIMEOUT_SPINS,
            true,
        )
    }

    fn transfer_with_options(
        &mut self,
        address: u8,
        endpoint: u8,
        max_packet_size: u16,
        direction: UhciTransferDirection,
        data_toggle: &mut bool,
        bytes: &mut [u8],
        timeout_spins: usize,
        require_full: bool,
    ) -> Result<usize, UhciTransferError> {
        if address == 0 || address > UHCI_MAX_DEVICES as u8 {
            return Err(UhciTransferError::InvalidAddress);
        }
        if endpoint > 15 {
            return Err(UhciTransferError::InvalidEndpoint);
        }
        let packet_size = usize::from(max_packet_size);
        if packet_size == 0 || packet_size > 2047 {
            return Err(UhciTransferError::InvalidPacketSize);
        }
        if bytes.len() > 4096 {
            return Err(UhciTransferError::TransferTooLarge);
        }
        let td_count = bytes.len().div_ceil(packet_size).max(1);
        if td_count > UHCI_MAX_TDS {
            return Err(UhciTransferError::TransferTooLarge);
        }
        let schedule = self
            .schedule
            .ok_or(UhciTransferError::ScheduleUnavailable)?;
        let virtual_offset = VirtAddr::new(schedule.physical_memory_offset);
        let descriptors = (virtual_offset + u64::from(schedule.transfer_descriptors))
            .as_mut_ptr::<UhciTransferDescriptor>();
        let data_pointer = (virtual_offset + u64::from(schedule.data_buffer)).as_mut_ptr::<u8>();
        let queue_head =
            (virtual_offset + u64::from(schedule.queue_head)).as_mut_ptr::<UhciQueueHead>();
        let mut next_toggle = *data_toggle;

        if direction == UhciTransferDirection::Out && !bytes.is_empty() {
            // SAFETY: the bounded source fits in the dedicated 4 KiB DMA page.
            unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), data_pointer, bytes.len()) };
        }

        for index in 0..td_count {
            let offset = index * packet_size;
            let length = bytes.len().saturating_sub(offset).min(packet_size);
            let link_pointer = if index + 1 == td_count {
                UHCI_LINK_TERMINATE
            } else {
                schedule.transfer_descriptors
                    + ((index + 1) * core::mem::size_of::<UhciTransferDescriptor>()) as u32
                    | UHCI_LINK_DEPTH_FIRST
            };
            let pid = match direction {
                UhciTransferDirection::In => USB_PID_IN,
                UhciTransferDirection::Out => USB_PID_OUT,
            };
            let descriptor = UhciTransferDescriptor {
                link_pointer,
                control_status: UHCI_TD_ACTIVE | UHCI_TD_ERROR_RETRIES,
                token: encode_td_token(pid, address, endpoint, next_toggle, length),
                buffer_pointer: if length == 0 {
                    0
                } else {
                    schedule.data_buffer + offset as u32
                },
            };
            // SAFETY: `index` is bounded by the one-page TD arena capacity.
            unsafe { descriptors.add(index).write_volatile(descriptor) };
            next_toggle = !next_toggle;
        }

        self.execute_td_chain(
            queue_head,
            descriptors,
            schedule.transfer_descriptors,
            td_count,
            timeout_spins,
        )?;

        let mut transferred = 0usize;
        for index in 0..td_count {
            // SAFETY: every descriptor in this range was initialized above.
            let status = unsafe {
                core::ptr::addr_of!((*descriptors.add(index)).control_status).read_volatile()
            };
            if status & UHCI_TD_ERROR_MASK != 0 {
                return Err(UhciTransferError::TransactionFault { status });
            }
            transferred = transferred.saturating_add(decode_actual_length(status));
        }
        let mut controller_status = Port::<u16>::new(self.iobase + UHCI_STATUS_REGISTER);
        // SAFETY: USBSTS is the status register of this controller. Writing
        // asserted bits acknowledges them after the polled transaction.
        let status = unsafe { controller_status.read() };
        if status
            & ((UhciStatus::HostSystemError as u16)
                | (UhciStatus::HostControllerProcessError as u16))
            != 0
        {
            unsafe { controller_status.write(status) };
            return Err(UhciTransferError::ControllerFault);
        }
        unsafe { controller_status.write(status) };
        if require_full && transferred != bytes.len() {
            return Err(UhciTransferError::ShortPacket);
        }
        if direction == UhciTransferDirection::In && !bytes.is_empty() {
            // SAFETY: the completed DMA transfer wrote within its bounded page.
            unsafe {
                core::ptr::copy_nonoverlapping(data_pointer, bytes.as_mut_ptr(), transferred)
            };
        }
        *data_toggle = next_toggle;
        Ok(transferred)
    }

    fn ensure_device_active(&self, address: u8) -> Result<(), UhciTransferError> {
        match self
            .mass_storage_devices
            .iter()
            .find(|device| device.address == address)
            .map(|device| device.state)
        {
            Some(UsbDeviceState::Ejected) => Err(UhciTransferError::DeviceEjected),
            Some(UsbDeviceState::Disconnected) => Err(UhciTransferError::DeviceDisconnected),
            Some(UsbDeviceState::Detected | UsbDeviceState::Ready) => Ok(()),
            None => Err(UhciTransferError::DeviceDisconnected),
        }
    }

    fn control_transfer(
        &mut self,
        address: u8,
        max_packet_size: u8,
        setup: [u8; 8],
        data_direction: Option<UhciTransferDirection>,
        data: &mut [u8],
    ) -> Result<usize, UhciTransferError> {
        if address > UHCI_MAX_DEVICES as u8 {
            return Err(UhciTransferError::InvalidAddress);
        }
        let packet_size = usize::from(max_packet_size);
        if packet_size == 0 || packet_size > 64 {
            return Err(UhciTransferError::InvalidPacketSize);
        }
        if data.len() > 4096 - USB_CONTROL_BUFFER_OFFSET {
            return Err(UhciTransferError::TransferTooLarge);
        }
        if data.is_empty() != data_direction.is_none() {
            return Err(UhciTransferError::TransferTooLarge);
        }
        let data_td_count = data.len().div_ceil(packet_size);
        let td_count = data_td_count + 2;
        if td_count > UHCI_MAX_TDS {
            return Err(UhciTransferError::TransferTooLarge);
        }
        let schedule = self
            .schedule
            .ok_or(UhciTransferError::ScheduleUnavailable)?;
        let virtual_offset = VirtAddr::new(schedule.physical_memory_offset);
        let descriptors = (virtual_offset + u64::from(schedule.transfer_descriptors))
            .as_mut_ptr::<UhciTransferDescriptor>();
        let data_pointer = (virtual_offset + u64::from(schedule.data_buffer)).as_mut_ptr::<u8>();
        let queue_head =
            (virtual_offset + u64::from(schedule.queue_head)).as_mut_ptr::<UhciQueueHead>();

        // SAFETY: setup plus bounded data fit inside the dedicated DMA page.
        unsafe {
            core::ptr::copy_nonoverlapping(setup.as_ptr(), data_pointer, setup.len());
            if data_direction == Some(UhciTransferDirection::Out) && !data.is_empty() {
                core::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    data_pointer.add(USB_CONTROL_BUFFER_OFFSET),
                    data.len(),
                );
            }
        }

        let setup_td = UhciTransferDescriptor {
            link_pointer: schedule.transfer_descriptors
                + core::mem::size_of::<UhciTransferDescriptor>() as u32
                | UHCI_LINK_DEPTH_FIRST,
            control_status: UHCI_TD_ACTIVE | UHCI_TD_ERROR_RETRIES,
            token: encode_td_token(USB_PID_SETUP, address, 0, false, 8),
            buffer_pointer: schedule.data_buffer,
        };
        // SAFETY: TD zero is within the dedicated descriptor arena.
        unsafe { descriptors.write_volatile(setup_td) };

        let mut toggle = true;
        for index in 0..data_td_count {
            let offset = index * packet_size;
            let length = data.len().saturating_sub(offset).min(packet_size);
            let td_index = index + 1;
            let pid = match data_direction.expect("data direction checked above") {
                UhciTransferDirection::In => USB_PID_IN,
                UhciTransferDirection::Out => USB_PID_OUT,
            };
            let descriptor = UhciTransferDescriptor {
                link_pointer: schedule.transfer_descriptors
                    + ((td_index + 1) * core::mem::size_of::<UhciTransferDescriptor>()) as u32
                    | UHCI_LINK_DEPTH_FIRST,
                control_status: UHCI_TD_ACTIVE | UHCI_TD_ERROR_RETRIES,
                token: encode_td_token(pid, address, 0, toggle, length),
                buffer_pointer: schedule.data_buffer + (USB_CONTROL_BUFFER_OFFSET + offset) as u32,
            };
            // SAFETY: the prevalidated TD count fits in the arena.
            unsafe { descriptors.add(td_index).write_volatile(descriptor) };
            toggle = !toggle;
        }

        let status_pid = match data_direction {
            Some(UhciTransferDirection::In) => USB_PID_OUT,
            Some(UhciTransferDirection::Out) | None => USB_PID_IN,
        };
        let status_td = UhciTransferDescriptor {
            link_pointer: UHCI_LINK_TERMINATE,
            control_status: UHCI_TD_ACTIVE | UHCI_TD_ERROR_RETRIES,
            token: encode_td_token(status_pid, address, 0, true, 0),
            buffer_pointer: 0,
        };
        // SAFETY: the status TD is the final prevalidated descriptor.
        unsafe { descriptors.add(td_count - 1).write_volatile(status_td) };

        self.execute_td_chain(
            queue_head,
            descriptors,
            schedule.transfer_descriptors,
            td_count,
            UHCI_TRANSFER_TIMEOUT_SPINS,
        )?;

        let mut transferred = 0;
        for index in 0..data_td_count {
            // SAFETY: these data TDs completed successfully in the chain.
            let status = unsafe {
                core::ptr::addr_of!((*descriptors.add(index + 1)).control_status).read_volatile()
            };
            transferred += decode_actual_length(status);
        }
        if transferred != data.len() {
            return Err(UhciTransferError::ShortPacket);
        }
        if data_direction == Some(UhciTransferDirection::In) && !data.is_empty() {
            // SAFETY: completed IN data occupies the bounded control data area.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    data_pointer.add(USB_CONTROL_BUFFER_OFFSET),
                    data.as_mut_ptr(),
                    data.len(),
                )
            };
        }
        Ok(transferred)
    }

    fn execute_td_chain(
        &self,
        queue_head: *mut UhciQueueHead,
        descriptors: *mut UhciTransferDescriptor,
        first_descriptor: u32,
        td_count: usize,
        timeout_spins: usize,
    ) -> Result<(), UhciTransferError> {
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        // SAFETY: the complete chain is visible before publishing its head.
        unsafe {
            core::ptr::addr_of_mut!((*queue_head).element_link_pointer)
                .write_volatile(first_descriptor);
        }
        let mut completed = false;
        for _ in 0..timeout_spins {
            // SAFETY: the final descriptor is within the initialized chain.
            let status = unsafe {
                core::ptr::addr_of!((*descriptors.add(td_count - 1)).control_status).read_volatile()
            };
            if status & UHCI_TD_ACTIVE == 0 {
                completed = true;
                break;
            }
            for index in 0..td_count {
                // SAFETY: each inspected TD belongs to this initialized chain.
                let status = unsafe {
                    core::ptr::addr_of!((*descriptors.add(index)).control_status).read_volatile()
                };
                if status & (UHCI_TD_ERROR_MASK & !(1 << 19)) != 0 {
                    // SAFETY: this QH is exclusively owned by the synchronous
                    // executor; terminating it prevents another hardware read.
                    unsafe {
                        core::ptr::addr_of_mut!((*queue_head).element_link_pointer)
                            .write_volatile(UHCI_LINK_TERMINATE)
                    };
                    return Err(UhciTransferError::TransactionFault { status });
                }
            }
            core::hint::spin_loop();
        }
        if !completed {
            // SAFETY: terminating the QH safely detaches the timed-out chain.
            unsafe {
                core::ptr::addr_of_mut!((*queue_head).element_link_pointer)
                    .write_volatile(UHCI_LINK_TERMINATE)
            };
            return Err(UhciTransferError::Timeout);
        }
        for index in 0..td_count {
            // SAFETY: every TD is inside the completed chain.
            let status = unsafe {
                core::ptr::addr_of!((*descriptors.add(index)).control_status).read_volatile()
            };
            if status & UHCI_TD_ERROR_MASK != 0 {
                return Err(UhciTransferError::TransactionFault { status });
            }
        }
        Ok(())
    }

    pub fn detect_devices(&mut self) -> Result<(), &'static str> {
        crate::serial_println!("[USB] Scanning for connected devices...");

        let mut next_address = 1u8;
        for port in 0..2 {
            let status = self.get_port_status(port);

            // Check if device is connected (bit 0: CurrentConnectStatus)
            if (status & 0x0001) != 0 {
                crate::serial_println!("[USB] Device detected on port {}", port);
                match self
                    .reset_port(port)
                    .and_then(|_| self.enumerate_device(port, next_address))
                {
                    Ok(device) => {
                        crate::serial_println!(
                            "[USB] enumerated port={} addr={} vid={:04x} pid={:04x}",
                            port,
                            device.address,
                            device.vendor_id,
                            device.product_id
                        );
                        let first_new_mass_storage = self
                            .mass_storage_devices
                            .iter()
                            .position(|candidate| candidate.address == device.address);
                        if let Some(index) = first_new_mass_storage {
                            match self.probe_mass_storage(index) {
                                Ok(()) => {}
                                Err(error) => {
                                    crate::serial_println!(
                                        "[USB-MASS] probe failed addr={}: {:?}",
                                        device.address,
                                        error
                                    );
                                }
                            }
                        }
                        next_address = next_address.saturating_add(1);
                    }
                    Err(error) => {
                        crate::serial_println!(
                            "[USB] enumeration failed port={}: {:?}",
                            port,
                            error
                        );
                    }
                }
            }
        }

        Ok(())
    }

    fn reset_port(&mut self, port: u8) -> Result<(), UsbEnumerationError> {
        if port >= 2 {
            return Err(UsbEnumerationError::PortReset);
        }
        let mut port_register = Port::<u16>::new(self.iobase + 0x10 + u16::from(port) * 2);
        // SAFETY: PORTSC belongs to this UHCI controller. Reset is asserted
        // and released while enumeration is single-threaded.
        unsafe {
            let status = port_register.read();
            port_register.write(status | UHCI_PORT_RESET);
        }
        crate::timer::sleep_ms(50);
        unsafe {
            let status = port_register.read();
            port_register.write(
                (status & !UHCI_PORT_RESET)
                    | UHCI_PORT_ENABLED
                    | UHCI_PORT_CONNECT_CHANGED
                    | UHCI_PORT_ENABLE_CHANGED,
            );
        }
        crate::timer::sleep_ms(10);
        for _ in 0..1000 {
            let status = unsafe { port_register.read() };
            if status & UHCI_PORT_CONNECTED == 0 {
                return Err(UsbEnumerationError::PortReset);
            }
            if status & UHCI_PORT_ENABLED != 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(UsbEnumerationError::PortReset)
    }

    pub fn enumerate_device(
        &mut self,
        port: u8,
        address: u8,
    ) -> Result<UsbDevice, UsbEnumerationError> {
        crate::serial_println!("[USB] Enumerating device at address {}", address);
        if address == 0 || address > UHCI_MAX_DEVICES as u8 {
            return Err(UsbEnumerationError::InvalidDescriptor);
        }

        let mut first_descriptor = [0u8; 8];
        self.control_transfer(
            0,
            8,
            descriptor_request(USB_DESCRIPTOR_DEVICE, 0, first_descriptor.len() as u16),
            Some(UhciTransferDirection::In),
            &mut first_descriptor,
        )?;
        let max_packet_size = first_descriptor[7];
        if !matches!(max_packet_size, 8 | 16 | 32 | 64) {
            return Err(UsbEnumerationError::InvalidDescriptor);
        }

        let mut device_descriptor = [0u8; 18];
        self.control_transfer(
            0,
            max_packet_size,
            descriptor_request(USB_DESCRIPTOR_DEVICE, 0, device_descriptor.len() as u16),
            Some(UhciTransferDirection::In),
            &mut device_descriptor,
        )?;
        parse_device_descriptor(&device_descriptor)?;

        self.control_transfer(
            0,
            max_packet_size,
            control_setup(0, USB_REQUEST_SET_ADDRESS, u16::from(address), 0, 0),
            None,
            &mut [],
        )?;
        crate::timer::sleep_ms(10);

        let mut configuration_header = [0u8; 9];
        self.control_transfer(
            address,
            max_packet_size,
            descriptor_request(
                USB_DESCRIPTOR_CONFIGURATION,
                0,
                configuration_header.len() as u16,
            ),
            Some(UhciTransferDirection::In),
            &mut configuration_header,
        )?;
        if configuration_header[0] < 9 || configuration_header[1] != 2 {
            return Err(UsbEnumerationError::InvalidDescriptor);
        }
        let total_length = usize::from(u16::from_le_bytes([
            configuration_header[2],
            configuration_header[3],
        ]));
        if !(9..=4096 - USB_CONTROL_BUFFER_OFFSET).contains(&total_length) {
            return Err(UsbEnumerationError::InvalidDescriptor);
        }
        let mut configuration = alloc::vec![0u8; total_length];
        self.control_transfer(
            address,
            max_packet_size,
            descriptor_request(USB_DESCRIPTOR_CONFIGURATION, 0, total_length as u16),
            Some(UhciTransferDirection::In),
            &mut configuration,
        )?;
        let configuration_value = configuration[5];
        if configuration_value == 0 {
            return Err(UsbEnumerationError::InvalidDescriptor);
        }
        self.control_transfer(
            address,
            max_packet_size,
            control_setup(
                0,
                USB_REQUEST_SET_CONFIGURATION,
                u16::from(configuration_value),
                0,
                0,
            ),
            None,
            &mut [],
        )?;

        let device = self
            .register_descriptor_set(port, address, &device_descriptor, &configuration)
            .map_err(UsbEnumerationError::Descriptor)?;
        self.configure_hid_interfaces(address, max_packet_size)?;
        Ok(device)
    }

    pub fn register_descriptor_set(
        &mut self,
        port: u8,
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

        match detect_interface(configuration_bytes) {
            Ok(Some(interface)) => {
                crate::serial_println!(
                    "[USB-MASS] detected addr={} interface={} bulk-in=0x{:02x}/{} bulk-out=0x{:02x}/{} protocol=08/06/50",
                    address,
                    interface.interface_number,
                    interface.bulk_in_endpoint,
                    interface.bulk_in_max_packet_size,
                    interface.bulk_out_endpoint,
                    interface.bulk_out_max_packet_size,
                );
                self.mass_storage_devices.push(UsbMassStorageDevice {
                    port,
                    address,
                    vendor_id: device.vendor_id,
                    product_id: device.product_id,
                    interface,
                    control_max_packet_size: device.max_packet_size,
                    ready: false,
                    block_count: 0,
                    block_size: 0,
                    lba0_checksum: 0,
                    state: UsbDeviceState::Detected,
                });
            }
            Ok(None) => {}
            Err(UsbMassStorageDetectionError::MissingBulkEndpoints) => {
                crate::serial_println!(
                    "[USB-MASS] rejected addr={}: interface 08/06/50 lacks Bulk IN/OUT endpoints",
                    address
                );
            }
            Err(UsbMassStorageDetectionError::Descriptor(error)) => return Err(error),
        }
        for profile in detect_hid_interfaces(configuration_bytes)? {
            crate::serial_println!(
                "[USB-HID] detected addr={} interface={} type={:?} interrupt-in=0x{:02x}/{} interval={}ms",
                address,
                profile.interface_number,
                profile.hid_type,
                profile.interrupt_endpoint,
                profile.max_packet_size,
                profile.interval
            );
            self.hid_devices.push(UsbHidRuntimeDevice {
                port,
                address,
                interface_number: profile.interface_number,
                hid_type: profile.hid_type,
                interrupt_endpoint: profile.interrupt_endpoint,
                max_packet_size: profile.max_packet_size,
                interval: profile.interval,
                data_toggle: false,
                last_poll_tick: 0,
                state: UsbDeviceState::Detected,
            });
        }
        self.devices.push(device.clone());
        Ok(device)
    }

    fn configure_hid_interfaces(
        &mut self,
        address: u8,
        control_max_packet_size: u8,
    ) -> Result<(), UhciTransferError> {
        let interfaces: Vec<_> = self
            .hid_devices
            .iter()
            .filter(|device| device.address == address)
            .map(|device| device.interface_number)
            .collect();
        for interface in interfaces {
            self.control_transfer(
                address,
                control_max_packet_size,
                control_setup(0x21, 0x0b, 0, u16::from(interface), 0),
                None,
                &mut [],
            )?;
            self.control_transfer(
                address,
                control_max_packet_size,
                control_setup(0x21, 0x0a, 0, u16::from(interface), 0),
                None,
                &mut [],
            )?;
            if let Some(device) = self
                .hid_devices
                .iter_mut()
                .find(|device| device.address == address && device.interface_number == interface)
            {
                device.state = UsbDeviceState::Ready;
                crate::serial_println!(
                    "[USB-HID] polling ready addr={} interface={} type={:?}",
                    address,
                    interface,
                    device.hid_type
                );
            }
        }
        Ok(())
    }

    fn probe_mass_storage(&mut self, index: usize) -> Result<(), ScsiError<UhciTransferError>> {
        let descriptor = self.mass_storage_devices[index];
        let pipe = UhciControllerPipe {
            controller: self,
            address: descriptor.address,
            interface_number: descriptor.interface.interface_number,
            control_max_packet_size: descriptor.control_max_packet_size,
            bulk_in_endpoint: descriptor.interface.bulk_in_endpoint,
            bulk_in_max_packet_size: descriptor.interface.bulk_in_max_packet_size,
            bulk_out_endpoint: descriptor.interface.bulk_out_endpoint,
            bulk_out_max_packet_size: descriptor.interface.bulk_out_max_packet_size,
            in_toggle: false,
            out_toggle: false,
        };
        let transport = BulkOnlyTransport::new(
            pipe,
            descriptor.interface.bulk_in_endpoint,
            descriptor.interface.bulk_out_endpoint,
        );
        let mut scsi = UsbScsiDevice::new(transport, 0);
        let inquiry = scsi.inquiry()?;
        if inquiry.peripheral_qualifier != 0 || inquiry.peripheral_device_type != 0 {
            return Err(ScsiError::UnsupportedDevice);
        }
        scsi.test_unit_ready()?;
        let capacity = scsi.read_capacity_10()?;
        if capacity.block_size != 512 {
            return Err(ScsiError::UnsupportedBlockSize);
        }
        let mut block = [0u8; 512];
        scsi.read_10(0, 1, &mut block)?;
        core::mem::drop(scsi);
        let checksum = block.iter().fold(0u32, |sum, byte| {
            sum.rotate_left(5).wrapping_add(u32::from(*byte))
        });
        let device = &mut self.mass_storage_devices[index];
        device.ready = true;
        device.state = UsbDeviceState::Ready;
        device.block_count = capacity.block_count;
        device.block_size = capacity.block_size;
        device.lba0_checksum = checksum;
        crate::serial_println!(
            "[USB-MASS] ready addr={} vendor={} product={} blocks={} block-size={} lba0-checksum={:08x}",
            device.address,
            inquiry.vendor_str(),
            inquiry.product_str(),
            device.block_count,
            device.block_size,
            device.lba0_checksum
        );
        Ok(())
    }

    pub fn get_devices(&self) -> &[UsbDevice] {
        &self.devices
    }

    pub fn get_mass_storage_devices(&self) -> &[UsbMassStorageDevice] {
        &self.mass_storage_devices
    }

    fn poll_device_lifecycle(&mut self, now: u64) {
        if now.saturating_sub(self.last_port_poll_tick) < 10 {
            return;
        }
        self.last_port_poll_tick = now;
        let connected = [
            self.get_port_status(0) & UHCI_PORT_CONNECTED != 0,
            self.get_port_status(1) & UHCI_PORT_CONNECTED != 0,
        ];
        for device in &mut self.mass_storage_devices {
            if !connected[usize::from(device.port)]
                && !matches!(
                    device.state,
                    UsbDeviceState::Disconnected | UsbDeviceState::Ejected
                )
            {
                device.ready = false;
                device.state = UsbDeviceState::Disconnected;
                crate::serial_println!(
                    "[USB-MASS] disconnected port={} addr={}; future I/O rejected",
                    device.port,
                    device.address
                );
            }
        }
        for device in &mut self.hid_devices {
            if !connected[usize::from(device.port)]
                && !matches!(
                    device.state,
                    UsbDeviceState::Disconnected | UsbDeviceState::Ejected
                )
            {
                device.state = UsbDeviceState::Disconnected;
                crate::serial_println!(
                    "[USB-HID] disconnected port={} addr={} interface={}",
                    device.port,
                    device.address,
                    device.interface_number
                );
            }
        }
    }

    fn poll_hid_devices(&mut self, now: u64) {
        for index in 0..self.hid_devices.len() {
            let device = self.hid_devices[index];
            let interval_ticks = (u64::from(device.interval) * u64::from(crate::timer::TIMER_HZ))
                .div_ceil(1_000)
                .max(1);
            if device.state != UsbDeviceState::Ready
                || now.saturating_sub(device.last_poll_tick) < interval_ticks
            {
                continue;
            }
            self.hid_devices[index].last_poll_tick = now;
            let report_size = usize::from(device.max_packet_size).min(64);
            let mut report = [0u8; 64];
            let mut toggle = device.data_toggle;
            match self.transfer_with_options(
                device.address,
                device.interrupt_endpoint & 0x0f,
                device.max_packet_size,
                UhciTransferDirection::In,
                &mut toggle,
                &mut report[..report_size],
                UHCI_INTERRUPT_POLL_SPINS,
                false,
            ) {
                Ok(length) if length > 0 => {
                    self.hid_devices[index].data_toggle = toggle;
                    match device.hid_type {
                        HidDeviceType::Keyboard => {
                            process_keyboard_report(&HidKeyboardReport::new(&report[..length]));
                        }
                        HidDeviceType::Mouse => {
                            process_mouse_report(&HidMouseReport::new(&report[..length]));
                        }
                        HidDeviceType::Unknown => {}
                    }
                    crate::serial_println!(
                        "[USB-HID] report addr={} type={:?} bytes={}",
                        device.address,
                        device.hid_type,
                        length
                    );
                }
                Ok(_) | Err(UhciTransferError::Timeout) => {}
                Err(UhciTransferError::TransactionFault { status })
                    if status & UHCI_TD_ERROR_MASK == 1 << 19 => {}
                Err(error) => {
                    crate::serial_println!(
                        "[USB-HID] poll error addr={} type={:?}: {:?}",
                        device.address,
                        device.hid_type,
                        error
                    );
                }
            }
        }
    }
}

fn detect_hid_interfaces(
    configuration_bytes: &[u8],
) -> Result<Vec<HidInterfaceProfile>, UsbDescriptorError> {
    let mut profiles = Vec::new();
    let mut current = None;
    for descriptor in DescriptorIter::new(configuration_bytes) {
        match descriptor? {
            ParsedDescriptor::Interface(interface) => {
                let hid_type = if interface.interface_class == 0x03 {
                    HidDeviceType::from_interface_subclass(
                        interface.interface_subclass,
                        interface.interface_protocol,
                    )
                } else {
                    HidDeviceType::Unknown
                };
                current = (hid_type != HidDeviceType::Unknown)
                    .then_some((interface.interface_number, hid_type));
            }
            ParsedDescriptor::Endpoint(endpoint) => {
                let Some((interface_number, hid_type)) = current else {
                    continue;
                };
                if endpoint.address & 0x80 == 0
                    || endpoint.attributes & 0x03 != 0x03
                    || endpoint.max_packet_size == 0
                    || endpoint.max_packet_size > 64
                {
                    continue;
                }
                profiles.push(HidInterfaceProfile {
                    interface_number,
                    hid_type,
                    interrupt_endpoint: endpoint.address,
                    max_packet_size: endpoint.max_packet_size,
                    interval: endpoint.interval.max(1),
                });
                current = None;
            }
            ParsedDescriptor::Configuration(_) | ParsedDescriptor::Unknown { .. } => {}
        }
    }
    Ok(profiles)
}

lazy_static! {
    pub static ref UHCI_CONTROLLER: Mutex<Option<UhciController>> = Mutex::new(None);
}

pub fn init_usb(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    physical_memory_offset: VirtAddr,
) {
    crate::serial_println!("[USB] Initializing USB subsystem...");

    let Some((pci_device, iobase)) = find_uhci_iobase() else {
        crate::drivers::status::report(
            "usb-uhci",
            crate::drivers::status::DriverState::Missing,
            "controller not found",
        );
        crate::serial_println!("[USB] No UHCI controller found through PCI");
        return;
    };

    let mut controller = UhciController::new(iobase);
    pci::enable_io_and_bus_master(pci_device);
    match controller
        .init()
        .and_then(|_| controller.initialize_schedule(frame_allocator, physical_memory_offset))
        .and_then(|_| controller.enable())
    {
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

fn find_uhci_iobase() -> Option<(pci::PciDevice, u16)> {
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
    Some((device, iobase))
}

fn allocate_uhci_frame(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<u32, &'static str> {
    let physical = frame_allocator
        .allocate_frame()
        .ok_or("UHCI DMA frame allocation failed")?
        .start_address()
        .as_u64();
    u32::try_from(physical).map_err(|_| "UHCI requires DMA frames below 4 GiB")
}

fn encode_td_token(pid: u8, address: u8, endpoint: u8, data_toggle: bool, length: usize) -> u32 {
    let encoded_length = if length == 0 {
        0x7ff
    } else {
        (length - 1) as u32
    };
    u32::from(pid)
        | (u32::from(address) << 8)
        | (u32::from(endpoint) << 15)
        | (u32::from(data_toggle) << 19)
        | (encoded_length << 21)
}

fn decode_actual_length(control_status: u32) -> usize {
    let encoded = control_status & 0x7ff;
    if encoded == 0x7ff {
        0
    } else {
        encoded as usize + 1
    }
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

fn descriptor_request(descriptor_type: u16, index: u8, length: u16) -> [u8; 8] {
    control_setup(
        0x80,
        USB_REQUEST_GET_DESCRIPTOR,
        (descriptor_type << 8) | u16::from(index),
        0,
        length,
    )
}

fn reset_mass_storage_endpoints(
    controller: &mut UhciController,
    address: u8,
    control_max_packet_size: u8,
    interface_number: u8,
    bulk_in_endpoint: u8,
    bulk_out_endpoint: u8,
) -> Result<(), UhciTransferError> {
    controller.control_transfer(
        address,
        control_max_packet_size,
        control_setup(
            0x21,
            USB_MASS_STORAGE_RESET,
            0,
            u16::from(interface_number),
            0,
        ),
        None,
        &mut [],
    )?;
    for endpoint in [bulk_in_endpoint, bulk_out_endpoint] {
        controller.control_transfer(
            address,
            control_max_packet_size,
            control_setup(0x02, USB_REQUEST_CLEAR_FEATURE, 0, u16::from(endpoint), 0),
            None,
            &mut [],
        )?;
    }
    Ok(())
}

pub fn get_devices() -> usize {
    UHCI_CONTROLLER
        .lock()
        .as_ref()
        .map(|c| c.get_devices().len())
        .unwrap_or(0)
}

pub fn get_mass_storage_devices() -> Vec<UsbMassStorageDevice> {
    UHCI_CONTROLLER
        .lock()
        .as_ref()
        .map(|controller| controller.get_mass_storage_devices().to_vec())
        .unwrap_or_default()
}

pub fn open_mass_storage_pipe(index: usize) -> Result<UhciMassStoragePipe, UhciTransferError> {
    let controller = UHCI_CONTROLLER.lock();
    let device = controller
        .as_ref()
        .and_then(|controller| controller.get_mass_storage_devices().get(index))
        .ok_or(UhciTransferError::ScheduleUnavailable)?;
    match device.state {
        UsbDeviceState::Ready => {}
        UsbDeviceState::Ejected => return Err(UhciTransferError::DeviceEjected),
        UsbDeviceState::Detected | UsbDeviceState::Disconnected => {
            return Err(UhciTransferError::DeviceDisconnected);
        }
    }
    Ok(UhciMassStoragePipe {
        address: device.address,
        interface_number: device.interface.interface_number,
        control_max_packet_size: device.control_max_packet_size,
        bulk_in_endpoint: device.interface.bulk_in_endpoint,
        bulk_in_max_packet_size: device.interface.bulk_in_max_packet_size,
        bulk_out_endpoint: device.interface.bulk_out_endpoint,
        bulk_out_max_packet_size: device.interface.bulk_out_max_packet_size,
        in_toggle: false,
        out_toggle: false,
    })
}

pub fn eject_mass_storage(index: usize) -> Result<(), UhciTransferError> {
    let mut controller = UHCI_CONTROLLER.lock();
    let device = controller
        .as_mut()
        .and_then(|controller| controller.mass_storage_devices.get_mut(index))
        .ok_or(UhciTransferError::DeviceDisconnected)?;
    match device.state {
        UsbDeviceState::Ready => {
            device.ready = false;
            device.state = UsbDeviceState::Ejected;
            crate::serial_println!(
                "[USB-MASS] ejected port={} addr={}; handles invalidated",
                device.port,
                device.address
            );
            Ok(())
        }
        UsbDeviceState::Ejected => Err(UhciTransferError::DeviceEjected),
        UsbDeviceState::Detected | UsbDeviceState::Disconnected => {
            Err(UhciTransferError::DeviceDisconnected)
        }
    }
}

pub fn poll_runtime() {
    if let Some(controller) = UHCI_CONTROLLER.lock().as_mut() {
        let now = crate::timer::ticks();
        controller.poll_device_lifecycle(now);
        controller.poll_hid_devices(now);
    }
}

#[cfg(feature = "usb-write-test")]
pub fn run_write_test() {
    const MAGIC: &[u8] = b"VANTARA_USB_WRITE_V1";
    let result = (|| {
        let descriptor = get_mass_storage_devices().first().copied()?;
        let pipe = open_mass_storage_pipe(0).ok()?;
        let transport = BulkOnlyTransport::new(
            pipe,
            descriptor.interface.bulk_in_endpoint,
            descriptor.interface.bulk_out_endpoint,
        );
        let scsi = UsbScsiDevice::new(transport, 0);
        let mut device = UsbMassStorageWritableBlockDevice::probe(scsi).ok()?;
        let lba = device.block_count().checked_sub(1)?;
        let mut marker = [0u8; BLOCK_SIZE];
        marker[..MAGIC.len()].copy_from_slice(MAGIC);
        for (index, byte) in marker[MAGIC.len()..].iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(31).wrapping_add(0x43);
        }
        let mut existing = [0u8; BLOCK_SIZE];
        device.read_block(lba, &mut existing).ok()?;
        let phase = if existing == marker {
            "verify"
        } else {
            device.write_block(lba, &marker).ok()?;
            let mut readback = [0u8; BLOCK_SIZE];
            device.read_block(lba, &mut readback).ok()?;
            if readback != marker {
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
                "[USB-WRITE-TEST] phase={} lba={} checksum={} persisted=true flush=true",
                phase,
                lba,
                checksum
            );
        }
        None => {
            crate::serial_println!("[USB-WRITE-TEST] failed");
        }
    }
}

pub fn write_mass_storage_to_buffer(out: &mut [u8]) -> usize {
    let controller = UHCI_CONTROLLER.lock();
    let mut writer = UsbMassStorageWriter::new(out);
    if let Some(schedule) = controller
        .as_ref()
        .and_then(|controller| controller.schedule.as_ref())
    {
        writer.write_str("UHCI schedule frame-list=0x");
        writer.write_hex(u64::from(schedule.frame_list), 8);
        writer.write_str(" qh=0x");
        writer.write_hex(u64::from(schedule.queue_head), 8);
        writer.write_str(" td=0x");
        writer.write_hex(u64::from(schedule.transfer_descriptors), 8);
        writer.write_str(" data=0x");
        writer.write_hex(u64::from(schedule.data_buffer), 8);
        writer.write_str(" phys-offset=0x");
        writer.write_hex(schedule.physical_memory_offset, 16);
        writer.write_byte(b'\n');
    } else {
        writer.write_str("UHCI schedule unavailable\n");
    }
    writer.write_str("ADDR VID:PID    IF BULK-IN BULK-OUT STATE BLOCKS SIZE LBA0\n");
    let devices = controller
        .as_ref()
        .map(|controller| controller.get_mass_storage_devices())
        .unwrap_or(&[]);
    for device in devices {
        writer.write_u64(u64::from(device.address));
        writer.write_str("    ");
        writer.write_hex(u64::from(device.vendor_id), 4);
        writer.write_byte(b':');
        writer.write_hex(u64::from(device.product_id), 4);
        writer.write_str("  ");
        writer.write_u64(u64::from(device.interface.interface_number));
        writer.write_str("  0x");
        writer.write_hex(u64::from(device.interface.bulk_in_endpoint), 2);
        writer.write_byte(b'/');
        writer.write_u64(u64::from(device.interface.bulk_in_max_packet_size));
        writer.write_str("  0x");
        writer.write_hex(u64::from(device.interface.bulk_out_endpoint), 2);
        writer.write_byte(b'/');
        writer.write_u64(u64::from(device.interface.bulk_out_max_packet_size));
        writer.write_str("  ");
        writer.write_str(match device.state {
            UsbDeviceState::Detected => "detected ",
            UsbDeviceState::Ready => "ready ",
            UsbDeviceState::Disconnected => "disconnected ",
            UsbDeviceState::Ejected => "ejected ",
        });
        writer.write_u64(device.block_count);
        writer.write_byte(b' ');
        writer.write_u64(u64::from(device.block_size));
        writer.write_str(" 0x");
        writer.write_hex(u64::from(device.lba0_checksum), 8);
        writer.write_byte(b'\n');
    }
    writer.len
}

struct UsbMassStorageWriter<'a> {
    out: &'a mut [u8],
    len: usize,
}

impl<'a> UsbMassStorageWriter<'a> {
    fn new(out: &'a mut [u8]) -> Self {
        Self { out, len: 0 }
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
    fn write_u64(&mut self, mut value: u64) {
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
            self.write_byte(*byte);
        }
    }
    fn write_hex(&mut self, value: u64, digits: usize) {
        for shift in (0..digits).rev() {
            let nibble = ((value >> (shift * 4)) & 0xf) as u8;
            self.write_byte(if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HidDeviceType, control_setup, decode_actual_length, descriptor_request,
        detect_hid_interfaces, encode_td_token,
    };

    #[test_case]
    fn encodes_uhci_token_fields_and_zero_length() {
        let token = encode_td_token(0x69, 5, 2, true, 64);
        assert_eq!(token & 0xff, 0x69);
        assert_eq!((token >> 8) & 0x7f, 5);
        assert_eq!((token >> 15) & 0x0f, 2);
        assert_eq!((token >> 19) & 1, 1);
        assert_eq!((token >> 21) & 0x7ff, 63);
        assert_eq!((encode_td_token(0xe1, 1, 0, false, 0) >> 21) & 0x7ff, 0x7ff);
    }

    #[test_case]
    fn decodes_uhci_actual_length_field() {
        assert_eq!(decode_actual_length(0x7ff), 0);
        assert_eq!(decode_actual_length(0), 1);
        assert_eq!(decode_actual_length(511), 512);
    }

    #[test_case]
    fn encodes_standard_control_setup_packets() {
        assert_eq!(descriptor_request(1, 0, 18), [0x80, 6, 0, 1, 0, 0, 18, 0]);
        assert_eq!(control_setup(0, 5, 7, 0, 0), [0, 5, 7, 0, 0, 0, 0, 0]);
    }

    #[test_case]
    fn detects_boot_hid_interrupt_endpoints() {
        let descriptors = [
            9, 2, 41, 0, 2, 1, 0, 0x80, 50, // configuration
            9, 4, 0, 0, 1, 3, 1, 1, 0, // keyboard interface
            7, 5, 0x81, 3, 8, 0, 10, // keyboard interrupt IN
            9, 4, 1, 0, 1, 3, 1, 2, 0, // mouse interface
            7, 5, 0x82, 3, 4, 0, 8, // mouse interrupt IN
        ];
        let profiles = detect_hid_interfaces(&descriptors).unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].hid_type, HidDeviceType::Keyboard);
        assert_eq!(profiles[0].interrupt_endpoint, 0x81);
        assert_eq!(profiles[1].hid_type, HidDeviceType::Mouse);
        assert_eq!(profiles[1].max_packet_size, 4);
    }
}
