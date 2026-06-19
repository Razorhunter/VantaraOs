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

mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, USER_BASE + 0x360
mov rdx, 6
int 0x80

mov rax, SYS_LISTDIR
mov rdi, USER_BASE + 0x366
mov rsi, 1
mov rdx, IO_BUF
mov r10, IO_BUF_LEN
int 0x80

mov rdx, rax
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, IO_BUF
int 0x80

mov rax, SYS_EXIT
xor rdi, rdi
int 0x80

jmp $
