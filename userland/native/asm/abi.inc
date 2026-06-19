BITS 64
%define USER_BASE 0x1000000
ORG USER_BASE

%define SYS_EXIT       1
%define SYS_WRITE      2
%define SYS_UPTIME     3
%define SYS_WHOAMI     6
%define SYS_LISTDIR    7
%define SYS_READ_FILE  8
%define SYS_OPEN       10
%define SYS_READ       11
%define SYS_CLOSE      12
%define SYS_EXEC       13
%define SYS_WAIT       14
%define SYS_ABI_INFO   28

%define ABI_VERSION_MAJOR 1
%define ABI_VERSION_MINOR 0

%define ERR_UNKNOWN_SYSCALL -1
%define ERR_INVALID_ARGUMENT -2
%define ERR_NOT_IMPLEMENTED  -3
%define ERR_NO_SUCH_PROCESS  -4
%define ERR_NOT_CHILD        -5
%define ERR_WOULD_BLOCK      -6

%define STDIN  0
%define STDOUT 1
%define STDERR 2

%define UPTIME_PREFIX (USER_BASE + 0x300)
%define NEWLINE       (USER_BASE + 0x30b)
%define UPTIME_BUF    (USER_BASE + 0x320)
%define ARG_PATH      (USER_BASE + 0x340)
%define ARG_LEN       (USER_BASE + 0x378)
%define IO_BUF        (USER_BASE + 0x380)
%define ERR_MSG       (USER_BASE + 0x3f0)

%define UPTIME_PREFIX_LEN 11
%define NEWLINE_LEN 1
%define UPTIME_BUF_LEN 32
%define IO_BUF_LEN 128
%define CAT_ERROR_LEN 15
%define LS_ERROR_LEN 14
