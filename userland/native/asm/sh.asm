%include "abi.inc"

%define SH_PROMPT      (USER_BASE + 0x420)
%define SH_INPUT       (USER_BASE + 0x430)
%define SH_PATH        (USER_BASE + 0x470)
%define SH_ARG         (USER_BASE + 0x490)

%define SH_INPUT_LEN   32

.shell_start:
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, PROMPT_MSG
mov rdx, PROMPT_MSG_LEN
int 0x80

xor r12, r12

.read_loop:
mov rax, SYS_READ
mov rdi, STDIN
mov rsi, SH_INPUT
add rsi, r12
mov rdx, 1
int 0x80

test rax, rax
jz .read_loop
js .exit_error

mov al, [abs SH_INPUT + r12]

cmp al, 8
je .backspace

cmp al, 10
je .dispatch

cmp r12, SH_INPUT_LEN - 1
jae .read_loop

mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, SH_INPUT
add rsi, r12
mov rdx, 1
int 0x80

inc r12
jmp .read_loop

.backspace:
test r12, r12
jz .read_loop
dec r12
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, BACKSPACE_MSG
mov rdx, BACKSPACE_MSG_LEN
int 0x80
jmp .read_loop

.dispatch:
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, NL_MSG
mov rdx, NL_MSG_LEN
int 0x80

mov byte [abs SH_INPUT + r12], 0
test r12, r12
jz .empty

cmp r12, 4
je .maybe_help_or_exit

cmp r12, 2
je .maybe_ls

cmp r12, 3
je .maybe_cat

cmp r12, 6
je .maybe_uptime_whoami

jmp .unknown

.maybe_help_or_exit:
cmp dword [abs SH_INPUT], 0x706c6568 ; help
je .help
cmp dword [abs SH_INPUT], 0x74697865 ; exit
je .exit_ok
jmp .unknown

.maybe_ls:
cmp word [abs SH_INPUT], 0x736c ; ls
je .exec_ls
jmp .unknown

.maybe_cat:
cmp word [abs SH_INPUT], 0x6163 ; ca
jne .unknown
cmp byte [abs SH_INPUT + 2], 't'
je .exec_cat
jmp .unknown

.maybe_uptime_whoami:
cmp dword [abs SH_INPUT], 0x69747075 ; upti
jne .maybe_whoami
cmp word [abs SH_INPUT + 4], 0x656d ; me
je .exec_uptime
jmp .unknown

.maybe_whoami:
cmp dword [abs SH_INPUT], 0x616f6877 ; whoa
jne .unknown
cmp word [abs SH_INPUT + 4], 0x696d ; mi
je .exec_whoami
jmp .unknown

.exec_ls:
mov rsi, PATH_LS
mov rdx, PATH_LS_LEN
mov r8, ARG_BIN
mov r9, ARG_BIN_LEN
jmp .exec_path_arg

.exec_cat:
mov rsi, PATH_CAT
mov rdx, PATH_CAT_LEN
mov r8, 0
mov r9, 0
jmp .exec_path_arg

.exec_uptime:
mov rsi, PATH_UPTIME
mov rdx, PATH_UPTIME_LEN
mov r8, 0
mov r9, 0
jmp .exec_path_arg

.exec_whoami:
mov rsi, PATH_WHOAMI
mov rdx, PATH_WHOAMI_LEN
mov r8, 0
mov r9, 0
jmp .exec_path_arg

.exec_path_arg:
mov rdi, SH_PATH
mov rcx, rdx
cld
rep movsb

test r9, r9
jz .exec_no_arg

mov rdi, SH_ARG
mov rsi, r8
mov rcx, r9
cld
rep movsb

mov rax, SYS_EXEC
mov rdi, SH_PATH
mov rsi, rdx
mov rdx, SH_ARG
mov r10, r9
int 0x80
jmp .exit_ok

.exec_no_arg:
mov rax, SYS_EXEC
mov rdi, SH_PATH
mov rsi, rdx
xor rdx, rdx
xor r10, r10
int 0x80

jmp .exit_ok

.help:
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, HELP_MSG
mov rdx, HELP_MSG_LEN
int 0x80
jmp .exit_ok

.empty:
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, EMPTY_MSG
mov rdx, EMPTY_MSG_LEN
int 0x80
jmp .exit_ok

.unknown:
mov rax, SYS_WRITE
mov rdi, STDOUT
mov rsi, UNKNOWN_MSG
mov rdx, UNKNOWN_MSG_LEN
int 0x80
jmp .exit_ok

.exit_ok:
mov rax, SYS_EXIT
xor rdi, rdi
int 0x80

.exit_error:
mov rax, SYS_EXIT
mov rdi, 1
int 0x80

jmp $

PROMPT_MSG:
db "vsh> "
PROMPT_MSG_LEN equ $ - PROMPT_MSG

HELP_MSG:
db "help ls cat uptime whoami exit"
HELP_MSG_LEN equ $ - HELP_MSG

UNKNOWN_MSG:
db "?", 10
UNKNOWN_MSG_LEN equ $ - UNKNOWN_MSG

EMPTY_MSG:
db ""
EMPTY_MSG_LEN equ $ - EMPTY_MSG

NL_MSG:
db 10
NL_MSG_LEN equ $ - NL_MSG

BACKSPACE_MSG:
db 8, " ", 8
BACKSPACE_MSG_LEN equ $ - BACKSPACE_MSG

PATH_LS:
db "/bin/ls"
PATH_LS_LEN equ $ - PATH_LS

PATH_CAT:
db "/bin/cat"
PATH_CAT_LEN equ $ - PATH_CAT

PATH_UPTIME:
db "uptime"
PATH_UPTIME_LEN equ $ - PATH_UPTIME

PATH_WHOAMI:
db "whoami"
PATH_WHOAMI_LEN equ $ - PATH_WHOAMI

ARG_BIN:
db "/bin"
ARG_BIN_LEN equ $ - ARG_BIN
