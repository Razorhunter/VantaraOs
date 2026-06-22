use core::arch::global_asm;

use crate::drivers::keyboard::{KeyboardDriver, Ps2Keyboard};
use crate::drivers::mouse::{MOUSE, MouseDriver};
use crate::{gdt, println};
use lazy_static::lazy_static;
use pic8259::ChainedPics;
use x86_64::PrivilegeLevel;
use x86_64::VirtAddr;
use x86_64::instructions::interrupts;
use x86_64::registers::control::Cr2;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

pub static PICS: crate::sync::PreemptMutex<ChainedPics> =
    crate::sync::PreemptMutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

unsafe extern "C" {
    fn syscall_interrupt_entry();
    fn timer_interrupt_entry();
}

// SAFETY: this entry stub preserves every general-purpose register, passes a
// pointer to the common no-error-code interrupt frame, aligns the temporary
// call stack for the SysV ABI, restores the original stack pointer, and returns
// only through `iretq`.
global_asm!(
    r#"
    .global timer_interrupt_entry
timer_interrupt_entry:
    cld
    push r15
    push r14
    push r13
    push r12
    push r11
    push r10
    push r9
    push r8
    push rbp
    push rdi
    push rsi
    push rdx
    push rcx
    push rbx
    push rax

    mov rdi, rsp
    mov rax, rsp
    and rsp, -16
    sub rsp, 16
    mov [rsp], rax
    call timer_interrupt_dispatch
    test rax, rax
    jnz 1f
    mov rsp, [rsp]
    jmp 2f
1:
    # Bit zero marks a never-run task. Fresh tasks need an ordinary entry jump;
    # resumed tasks carry a real CPU interrupt frame and return through iretq.
    test rax, 1
    jz 3f
    and rax, -2
    mov rsp, rax
    pop rax
    pop rbx
    pop rcx
    pop rdx
    pop rsi
    pop rdi
    pop rbp
    pop r8
    pop r9
    pop r10
    pop r11
    pop r12
    pop r13
    pop r14
    pop r15
    push qword ptr [rsp + 16]
    popfq
    mov rax, [rsp]
    add rsp, 24
    jmp rax
3:
    mov rsp, rax
    pop rax
    pop rbx
    pop rcx
    pop rdx
    pop rsi
    pop rdi
    pop rbp
    pop r8
    pop r9
    pop r10
    pop r11
    pop r12
    pop r13
    pop r14
    pop r15
    iretq
2:
    pop rax
    pop rbx
    pop rcx
    pop rdx
    pop rsi
    pop rdi
    pop rbp
    pop r8
    pop r9
    pop r10
    pop r11
    pop r12
    pop r13
    pop r14
    pop r15
    iretq
    "#
);

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.divide_error.set_handler_fn(divide_error_handler);
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        idt.general_protection_fault
            .set_handler_fn(general_protection_fault_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);

        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);

            idt[InterruptIndex::Timer.as_u8()]
                .set_handler_addr(VirtAddr::new(timer_interrupt_entry as *const () as u64));

            idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);

            idt[InterruptIndex::Mouse.as_u8()].set_handler_fn(mouse_interrupt_handler);
        }

        unsafe {
            idt[crate::user::syscall::SYSCALL_INTERRUPT]
                .set_handler_addr(VirtAddr::new(syscall_interrupt_entry as *const () as u64))
                .set_privilege_level(PrivilegeLevel::Ring3);
        }

        idt
    };
}

pub fn init_idt() {
    IDT.load();
}

pub fn enable() {
    interrupts::enable();
}

pub fn disable() {
    interrupts::disable();
}

pub fn are_enabled() -> bool {
    interrupts::are_enabled()
}

pub fn without_interrupts<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    interrupts::without_interrupts(f)
}

pub fn notify_end_of_interrupt(interrupt: InterruptIndex) {
    unsafe {
        PICS.lock().notify_end_of_interrupt(interrupt.as_u8());
    }
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn divide_error_handler(stack_frame: InterruptStackFrame) {
    if is_user_exception(&stack_frame) {
        contain_user_exception("divide-error", 0x0bad_0000, &stack_frame, None);
    }

    panic!("EXCEPTION: DIVIDE ERROR\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    panic!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    if is_user_exception(&stack_frame) {
        contain_user_exception("invalid-opcode", 0x0bad_0006, &stack_frame, None);
    }

    panic!("EXCEPTION: INVALID OPCODE\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    if is_user_exception(&stack_frame) {
        contain_user_exception(
            "general-protection",
            0x0bad_000d,
            &stack_frame,
            Some(error_code),
        );
    }

    panic!(
        "EXCEPTION: GENERAL PROTECTION FAULT\nerror={:#x}\n{:#?}",
        error_code, stack_frame
    );
}

fn is_user_exception(stack_frame: &InterruptStackFrame) -> bool {
    stack_frame.code_segment.rpl() == PrivilegeLevel::Ring3
}

fn contain_user_exception(
    kind: &str,
    status: u64,
    stack_frame: &InterruptStackFrame,
    error_code: Option<u64>,
) -> ! {
    let rip = stack_frame.instruction_pointer.as_u64();
    match crate::user::process::mark_current_user_exception(rip, status) {
        Some(report) => {
            crate::serial_println!(
                "[USER] killed pid={} name={} fault={} rip={:#x} error={:?} status={:#x}",
                report.pid,
                report.name,
                kind,
                report.rip,
                error_code,
                report.status
            );
        }
        None => {
            panic!(
                "user exception without current process: fault={} rip={:#x} error={:?}",
                kind, rip, error_code
            );
        }
    }
    crate::user::syscall::user_exit_landing();
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    if error_code.contains(PageFaultErrorCode::USER_MODE) {
        const USER_PAGE_FAULT_STATUS: u64 = 0x0bad_f00d;
        let addr = Cr2::read();
        let addr_u64 = match addr {
            Ok(addr) => addr.as_u64(),
            Err(raw) => raw.0,
        };
        let report = crate::user::process::mark_current_user_fault(
            addr_u64,
            stack_frame.instruction_pointer.as_u64(),
            USER_PAGE_FAULT_STATUS,
        );
        match report {
            Some(report) => {
                crate::serial_println!(
                    "[USER] killed pid={} name={} fault=page kind={:?} addr={:#x} rip={:#x} error={:?} status={:#x}",
                    report.pid,
                    report.name,
                    report.kind,
                    report.addr,
                    report.rip,
                    error_code,
                    report.status
                );
            }
            None => {
                crate::serial_println!(
                    "[USER] page fault with no current process: addr={:#x} rip={:?} error={:?}",
                    addr_u64,
                    stack_frame.instruction_pointer,
                    error_code
                );
            }
        }
        crate::user::syscall::user_exit_landing();
    }

    let kernel_fault_addr = match Cr2::read() {
        Ok(addr) => addr.as_u64(),
        Err(raw) => raw.0,
    };
    crate::serial_println!(
        "[KERNEL FAULT] page addr={:#x} rip={:#x} error_bits={:#x}",
        kernel_fault_addr,
        stack_frame.instruction_pointer.as_u64(),
        error_code.bits()
    );
    panic!(
        "EXCEPTION: PAGE FAULT\naccessed={:?}\nerror={:?}\n{:#?}",
        Cr2::read(),
        error_code,
        stack_frame
    );
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct TimerInterruptFrame {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub instruction_pointer: u64,
    pub code_segment: u64,
    pub cpu_flags: u64,
}

const _: () = {
    assert!(core::mem::size_of::<TimerInterruptFrame>() == 144);
    assert!(core::mem::offset_of!(TimerInterruptFrame, rax) == 0);
    assert!(core::mem::offset_of!(TimerInterruptFrame, r15) == 112);
    assert!(core::mem::offset_of!(TimerInterruptFrame, instruction_pointer) == 120);
    assert!(core::mem::offset_of!(TimerInterruptFrame, code_segment) == 128);
    assert!(core::mem::offset_of!(TimerInterruptFrame, cpu_flags) == 136);
};

impl TimerInterruptFrame {
    fn interrupted_user(&self) -> bool {
        self.code_segment & 0x3 == PrivilegeLevel::Ring3 as u64
    }

    fn user_resume_context(&self) -> crate::user::process::UserResumeContext {
        debug_assert!(self.interrupted_user());
        crate::user::process::UserResumeContext {
            rip: self.instruction_pointer,
            rsp: self.user_stack_pointer(),
            rflags: self.cpu_flags,
            rbx: self.rbx,
            rcx: self.rcx,
            rdx: self.rdx,
            rsi: self.rsi,
            rdi: self.rdi,
            rbp: self.rbp,
            r8: self.r8,
            r9: self.r9,
            r10: self.r10,
            r11: self.r11,
            r12: self.r12,
            r13: self.r13,
            r14: self.r14,
            r15: self.r15,
            rax: self.rax,
        }
    }

    fn apply_user_resume(&mut self, context: crate::user::process::UserResumeContext) {
        debug_assert!(self.interrupted_user());
        self.rax = context.rax;
        self.rbx = context.rbx;
        self.rcx = context.rcx;
        self.rdx = context.rdx;
        self.rsi = context.rsi;
        self.rdi = context.rdi;
        self.rbp = context.rbp;
        self.r8 = context.r8;
        self.r9 = context.r9;
        self.r10 = context.r10;
        self.r11 = context.r11;
        self.r12 = context.r12;
        self.r13 = context.r13;
        self.r14 = context.r14;
        self.r15 = context.r15;
        self.instruction_pointer = context.rip;
        self.cpu_flags = context.rflags | 0x202;
        self.set_user_stack_pointer(context.rsp);
    }

    fn user_stack_pointer(&self) -> u64 {
        // SAFETY: a privilege-changing interrupt frame contains RSP and SS
        // immediately after the common RIP/CS/RFLAGS words. Callers verify
        // CS.RPL == Ring3 before reaching this accessor.
        unsafe { *((self as *const Self).add(1).cast::<u64>()) }
    }

    fn set_user_stack_pointer(&mut self, rsp: u64) {
        // SAFETY: this is the same Ring3-only hardware word described above;
        // mutating it changes the stack restored by the eventual `iretq`.
        unsafe {
            *((self as *mut Self).add(1).cast::<u64>()) = rsp;
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn timer_interrupt_dispatch(frame: &mut TimerInterruptFrame) -> u64 {
    crate::timer::tick();
    crate::user::process::record_timer_preemption_check();

    if frame.interrupted_user() {
        let now = crate::timer::ticks();
        crate::user::process::wake_sleeping_processes(now);
        if let Some(resume) =
            crate::user::process::preempt_current_user(frame.user_resume_context())
        {
            let target_p4 = resume
                .p4_frame
                .or_else(crate::user::address_space::kernel_p4_frame);
            if let Some(target_p4) = target_p4 {
                if crate::user::address_space::active_p4_frame() != target_p4 {
                    // SAFETY: scheduler results only expose page tables owned
                    // by a live process (or the recorded kernel P4), and every
                    // user P4 retains the kernel mappings used by this handler.
                    unsafe {
                        crate::user::address_space::switch_to_p4_frame(target_p4)
                            .expect("scheduled user address space must be switchable");
                    }
                }
            }
            frame.apply_user_resume(resume.resume_context);
        }
    } else if let Some(next_frame) =
        crate::scheduler::SCHEDULER.preempt_from_timer(frame as *mut TimerInterruptFrame as u64)
    {
        notify_end_of_interrupt(InterruptIndex::Timer);
        return next_frame;
    }

    notify_end_of_interrupt(InterruptIndex::Timer);
    0
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    notify_end_of_interrupt(InterruptIndex::Keyboard);

    let driver = Ps2Keyboard::new();
    driver.on_interrupt();
}

extern "x86-interrupt" fn mouse_interrupt_handler(_stack_frame: InterruptStackFrame) {
    notify_end_of_interrupt(InterruptIndex::Mouse);
    MOUSE.on_interrupt();
}

#[test_case]
fn test_breakpoint_exception() {
    x86_64::instructions::interrupts::int3();
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
    Mouse = PIC_2_OFFSET + 4,
}

impl InterruptIndex {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}
