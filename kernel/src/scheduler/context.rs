use super::task::Context;

unsafe extern "C" {
    pub fn setup_first_task(context: *const Context) -> !;
    pub fn start_kernel_interrupt_frame(frame_rsp: u64) -> !;
}

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    r#"
.global setup_first_task
.global start_kernel_interrupt_frame

start_kernel_interrupt_frame:
    mov rsp, rdi
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

# void setup_first_task(const Context *context)
# rdi = context pointer
setup_first_task:
    mov rsp, [rdi + 0x88]
    push qword ptr [rdi + 0x78]
    push qword ptr [rdi + 0x70]
    push qword ptr [rdi + 0x48]
    push qword ptr [rdi + 0x50]
    push qword ptr [rdi + 0x80]
    popfq

    mov r15, [rdi + 0x00]
    mov r14, [rdi + 0x08]
    mov r13, [rdi + 0x10]
    mov r12, [rdi + 0x18]
    mov r11, [rdi + 0x20]
    mov r10, [rdi + 0x28]
    mov r9, [rdi + 0x30]
    mov r8, [rdi + 0x38]
    mov rbp, [rdi + 0x40]
    mov rdx, [rdi + 0x58]
    mov rcx, [rdi + 0x60]
    mov rbx, [rdi + 0x68]
    pop rsi
    pop rdi
    pop rax
    ret
"#
);
