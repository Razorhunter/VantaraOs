%include "abi.inc"

mov rax, SYS_OPEN
mov rdi, ARG_PATH
mov rsi, [abs ARG_LEN]
int 0x80

test rax, rax
js .error

mov r12, rax

.read_loop:
mov rax, SYS_READ
mov rdi, r12
mov rsi, IO_BUF
mov rdx, IO_BUF_LEN
int 0x80

test rax, rax
js .close_error
jz .done

mov rdx, rax
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, IO_BUF
int 0x80
jmp .read_loop

.done:
mov rax, SYS_CLOSE
mov rdi, r12
int 0x80

mov rax, SYS_EXIT
xor rdi, rdi
int 0x80

jmp $

.close_error:
mov rax, SYS_CLOSE
mov rdi, r12
int 0x80

.error:
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, ERR_MSG
mov rdx, CAT_ERROR_LEN
int 0x80

mov rax, SYS_EXIT
mov rdi, 1
int 0x80

jmp $
