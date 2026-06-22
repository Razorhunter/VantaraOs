BITS 64
%define USER_BASE 0x1000000
ORG USER_BASE

%define SYS_EXIT       1
%define SYS_WRITE      2
%define SYS_UPTIME     3
%define SYS_WHOAMI     6
%define SYS_LISTDIR    7
%define SYS_READ_FILE  8
%define SYS_STAT       9
%define SYS_OPEN       10
%define SYS_READ       11
%define SYS_CLOSE      12
%define SYS_EXEC       13
%define SYS_WAIT       14
%define SYS_ABI_INFO   28
%define SYS_THREAD_CREATE 29
%define SYS_THREAD_EXIT   30
%define SYS_THREAD_JOIN   31
%define SYS_THREAD_SPAWN  32
%define SYS_PIPE          33
%define SYS_EVENT_CREATE  34
%define SYS_EVENT_WAIT    35
%define SYS_EVENT_SIGNAL  36
%define SYS_EVENT_CLOSE   37
%define SYS_MSGQ_CREATE   38
%define SYS_MSGQ_SEND     39
%define SYS_MSGQ_RECV     40
%define SYS_MSGQ_CLOSE    41
%define SYS_SIGPROCMASK   42
%define SYS_SIGPENDING    43
%define SYS_SIGACTION     44
%define SYS_SIGRETURN     45
%define SYS_GETPGRP       46
%define SYS_SETPGID       47
%define SYS_GETSID        48
%define SYS_SETSID        49
%define SYS_CREATE        50
%define SYS_UNLINK        51
%define SYS_RENAME        52
%define SYS_MKDIR         53
%define SYS_RMDIR         54

%define ABI_VERSION_MAJOR 1
%define ABI_VERSION_MINOR 12

%define ERR_UNKNOWN_SYSCALL -1
%define ERR_INVALID_ARGUMENT -2
%define ERR_NOT_IMPLEMENTED  -3
%define ERR_NO_SUCH_PROCESS  -4
%define ERR_NOT_CHILD        -5
%define ERR_WOULD_BLOCK      -6
%define ERR_PERMISSION_DENIED -7

%define SIGINT  2
%define SIGTERM 15
%define SIGCONT 18
%define SIGTSTP 20

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
%define TOUCH_ERROR_LEN CAT_ERROR_LEN
%define WRITE_ERROR_LEN CAT_ERROR_LEN
%define RM_ERROR_LEN CAT_ERROR_LEN
%define MV_ERROR_LEN CAT_ERROR_LEN
%define MKDIR_ERROR_LEN CAT_ERROR_LEN
%define RMDIR_ERROR_LEN CAT_ERROR_LEN
%define CAT_NOT_FILE_LEN 16
