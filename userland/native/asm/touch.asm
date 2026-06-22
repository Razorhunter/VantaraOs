%include "abi.inc"

mov rax, SYS_CREATE
mov rdi, ARG_PATH
mov rsi, [abs ARG_LEN]
int 0x80

test rax, rax
js .error

mov rdi, rax
mov rax, SYS_CLOSE
int 0x80

mov rax, SYS_EXIT
xor rdi, rdi
int 0x80

.error:
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, ERR_MSG
mov rdx, TOUCH_ERROR_LEN
int 0x80

mov rax, SYS_EXIT
mov rdi, 1
int 0x80

jmp $
