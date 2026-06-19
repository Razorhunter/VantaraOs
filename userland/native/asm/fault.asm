%include "abi.inc"

mov rax, [abs 0x0]

mov rax, SYS_EXIT
xor rdi, rdi
int 0x80

jmp $
