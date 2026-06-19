%include "abi.inc"

mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, UPTIME_PREFIX
mov rdx, UPTIME_PREFIX_LEN
int 0x80

mov rax, SYS_UPTIME
mov rdi, UPTIME_BUF
mov rsi, UPTIME_BUF_LEN
int 0x80

mov rdx, rax
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, UPTIME_BUF
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
