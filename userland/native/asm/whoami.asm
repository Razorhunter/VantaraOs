%include "abi.inc"

mov rax, SYS_WHOAMI
mov rdi, IO_BUF
mov rsi, IO_BUF_LEN
int 0x80

test rax, rax
js .error

mov rdx, rax
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, IO_BUF
int 0x80

mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, NEWLINE
mov rdx, NEWLINE_LEN
int 0x80

mov rax, SYS_EXIT
xor rdi, rdi
int 0x80

jmp $

.error:
mov rax, SYS_EXIT
mov rdi, 1
int 0x80

jmp $
