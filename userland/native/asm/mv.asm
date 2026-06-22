%include "abi.inc"

mov rcx, [abs ARG_LEN]
xor r8, r8

.find_space:
cmp r8, rcx
jae .error
cmp byte [ARG_PATH + r8], ' '
je .rename
inc r8
jmp .find_space

.rename:
lea rdx, [ARG_PATH + r8 + 1]
mov r10, rcx
sub r10, r8
dec r10

mov rax, SYS_RENAME
mov rdi, ARG_PATH
mov rsi, r8
int 0x80

test rax, rax
js .error

mov rax, SYS_EXIT
xor rdi, rdi
int 0x80

.error:
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, ERR_MSG
mov rdx, MV_ERROR_LEN
int 0x80

mov rax, SYS_EXIT
mov rdi, 1
int 0x80

jmp $
