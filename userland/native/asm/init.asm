%include "abi.inc"

mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, INIT_MSG
mov rdx, INIT_MSG_LEN
int 0x80

mov rax, SYS_EXEC
mov rdi, LOGIN_PATH
mov rsi, LOGIN_PATH_LEN
xor rdx, rdx
xor r10, r10
int 0x80

test rax, rax
js .exec_failed

mov rax, SYS_EXIT
xor rdi, rdi
int 0x80

.exec_failed:
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, INIT_EXEC_ERROR
mov rdx, INIT_EXEC_ERROR_LEN
int 0x80

mov rax, SYS_EXIT
mov rdi, 1
int 0x80

jmp $

INIT_MSG:
db "Vantara init started", 10
INIT_MSG_LEN equ $ - INIT_MSG

LOGIN_PATH:
db "/bin/login"
LOGIN_PATH_LEN equ $ - LOGIN_PATH

INIT_EXEC_ERROR:
db "init: exec /bin/login failed", 10
INIT_EXEC_ERROR_LEN equ $ - INIT_EXEC_ERROR
