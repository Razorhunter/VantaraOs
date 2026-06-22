%include "abi.inc"

mov rcx, [abs ARG_LEN]
xor r12, r12

.find_space:
cmp r12, rcx
jae .error
cmp byte [ARG_PATH + r12], ' '
je .open
inc r12
jmp .find_space

.open:
mov rax, SYS_OPEN
mov rdi, ARG_PATH
mov rsi, r12
int 0x80

test rax, rax
jns .write

mov rax, SYS_CREATE
mov rdi, ARG_PATH
mov rsi, r12
int 0x80
test rax, rax
js .error

.write:
mov r13, rax
lea rsi, [ARG_PATH + r12 + 1]
mov rdx, rcx
sub rdx, r12
dec rdx
mov rax, SYS_WRITE
mov rdi, r13
int 0x80
test rax, rax
js .close_error

mov rax, SYS_CLOSE
mov rdi, r13
int 0x80

mov rax, SYS_EXIT
xor rdi, rdi
int 0x80

.close_error:
mov rax, SYS_CLOSE
mov rdi, r13
int 0x80

.error:
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, ERR_MSG
mov rdx, WRITE_ERROR_LEN
int 0x80

mov rax, SYS_EXIT
mov rdi, 1
int 0x80

jmp $
