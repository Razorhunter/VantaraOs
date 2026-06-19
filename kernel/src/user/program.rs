use alloc::collections::VecDeque;
use lazy_static::lazy_static;
use spin::Mutex;
use x86_64::VirtAddr;

static USER_SHELL_RESPAWN: Mutex<bool> = Mutex::new(false);

lazy_static! {
    static ref PENDING_PROGRAMS: Mutex<VecDeque<UserProgram>> = Mutex::new(VecDeque::new());
    static ref USER_SHELL_WAIT_REQUESTED: Mutex<VecDeque<UserShellWaitRequest>> =
        Mutex::new(VecDeque::new());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserShellWaitRequest {
    pub shell_pid: crate::user::process::Pid,
    pub child_pid: crate::user::process::Pid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserProgramArg {
    pub bytes: [u8; crate::user::ring3::FIRST_USER_ARG_PATH_LEN as usize],
    pub len: usize,
}

impl UserProgramArg {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; crate::user::ring3::FIRST_USER_ARG_PATH_LEN as usize],
            len: 0,
        }
    }

    pub fn from_path(path: &str) -> Result<Self, ProgramError> {
        if path.is_empty() || path.len() > crate::user::ring3::FIRST_USER_ARG_PATH_LEN as usize {
            return Err(ProgramError::ArgumentTooLong);
        }

        let mut arg = Self::empty();
        arg.bytes[..path.len()].copy_from_slice(path.as_bytes());
        arg.len = path.len();
        Ok(arg)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).unwrap_or("<invalid>")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobMode {
    Foreground,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserProgram {
    pub pid: crate::user::process::Pid,
    pub parent_pid: Option<crate::user::process::Pid>,
    pub name: &'static str,
    pub path: &'static str,
    pub entry: VirtAddr,
    pub stack_top: VirtAddr,
    pub arg: UserProgramArg,
    pub job_mode: JobMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramError {
    NotFound,
    ArgumentTooLong,
}

pub fn request_path(path: &str) -> Result<crate::user::process::Pid, ProgramError> {
    request_path_with_arg(path, None)
}

pub fn request_path_with_arg(
    path: &str,
    arg: Option<&str>,
) -> Result<crate::user::process::Pid, ProgramError> {
    request_path_with_arg_and_mode(path, arg, JobMode::Foreground)
}

pub fn request_path_background(
    path: &str,
    arg: Option<&str>,
) -> Result<crate::user::process::Pid, ProgramError> {
    request_path_with_arg_and_mode(path, arg, JobMode::Background)
}

pub fn request_path_with_arg_and_mode(
    path: &str,
    arg: Option<&str>,
    job_mode: JobMode,
) -> Result<crate::user::process::Pid, ProgramError> {
    let image = crate::user::images::normalize_path(path)
        .and_then(crate::user::images::find)
        .ok_or(ProgramError::NotFound)?;
    let arg = UserProgramArg::from_path(arg.unwrap_or(default_arg_for(image.name)))?;
    let pid = crate::user::process::reserve_pid();
    let parent_pid = crate::user::process::current_user_pid().or(Some(1));

    let program = UserProgram {
        pid,
        parent_pid,
        name: image.name,
        path: image.path,
        entry: VirtAddr::new(crate::user::ring3::FIRST_USER_ENTRY),
        stack_top: VirtAddr::new(crate::user::ring3::FIRST_USER_STACK_TOP),
        arg,
        job_mode,
    };
    let mut pending = PENDING_PROGRAMS.lock();
    pending.push_back(program);
    let pending_count = pending.len();
    crate::serial_println!(
        "[USER] queued program {} pid={} ppid={:?} mode={:?} pending={}",
        image.path,
        pid,
        parent_pid,
        job_mode,
        pending_count
    );
    Ok(pid)
}

pub fn request_demo() {
    request_path_with_arg("demo", Some("/")).ok();
}

pub fn take_pending() -> Option<UserProgram> {
    PENDING_PROGRAMS.lock().pop_front()
}

pub fn has_pending() -> bool {
    !PENDING_PROGRAMS.lock().is_empty()
}

pub fn pending_count() -> usize {
    PENDING_PROGRAMS.lock().len()
}

pub fn set_user_shell_respawn(enabled: bool) {
    *USER_SHELL_RESPAWN.lock() = enabled;
}

pub fn should_respawn_user_shell() -> bool {
    *USER_SHELL_RESPAWN.lock()
}

pub fn pending_pid() -> Option<crate::user::process::Pid> {
    PENDING_PROGRAMS.lock().front().map(|program| program.pid)
}

pub fn request_user_shell_wait(
    shell_pid: crate::user::process::Pid,
    child_pid: crate::user::process::Pid,
) {
    USER_SHELL_WAIT_REQUESTED
        .lock()
        .push_back(UserShellWaitRequest {
            shell_pid,
            child_pid,
        });
}

pub fn user_shell_wait_request() -> Option<UserShellWaitRequest> {
    USER_SHELL_WAIT_REQUESTED.lock().front().copied()
}

pub fn take_user_shell_wait_request() -> Option<UserShellWaitRequest> {
    USER_SHELL_WAIT_REQUESTED.lock().pop_front()
}

pub fn take_user_shell_wait_request_for_child(
    child_pid: crate::user::process::Pid,
) -> Option<UserShellWaitRequest> {
    let mut requests = USER_SHELL_WAIT_REQUESTED.lock();
    let index = requests
        .iter()
        .position(|request| request.child_pid == child_pid)?;
    requests.remove(index)
}

fn default_arg_for(name: &str) -> &'static str {
    match name {
        "cat" => "/README",
        _ => "/",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        JobMode, ProgramError, pending_count, request_path, request_path_background,
        request_path_with_arg, take_pending,
    };

    #[test_case]
    fn queues_program_by_bin_path() {
        let pid = request_path("/bin/ls").expect("program should queue");

        let program = take_pending().expect("program should be queued");
        assert_eq!(program.pid, pid);
        assert_eq!(program.path, "/bin/ls");
        assert_eq!(program.name, "ls");
        assert_eq!(program.arg.as_str(), "/");
        assert_eq!(program.job_mode, JobMode::Foreground);
    }

    #[test_case]
    fn queues_program_with_arg() {
        let pid = request_path_with_arg("cat", Some("/MOTD")).expect("program should queue");

        let program = take_pending().expect("program should be queued");
        assert_eq!(program.pid, pid);
        assert_eq!(program.path, "/bin/cat");
        assert_eq!(program.name, "cat");
        assert_eq!(program.arg.as_str(), "/MOTD");
    }

    #[test_case]
    fn rejects_unknown_program_path() {
        assert_eq!(request_path("/bin/missing"), Err(ProgramError::NotFound));
    }

    #[test_case]
    fn queues_background_program_with_job_mode() {
        let pid = request_path_background("/bin/uptime", None).expect("program should queue");

        let program = take_pending().expect("program should be queued");
        assert_eq!(program.pid, pid);
        assert_eq!(program.name, "uptime");
        assert_eq!(program.job_mode, JobMode::Background);
    }

    #[test_case]
    fn queues_multiple_programs_in_fifo_order() {
        let first = request_path("/bin/uptime").expect("first program should queue");
        let second = request_path("/bin/whoami").expect("second program should queue");

        assert_eq!(pending_count(), 2);
        assert_eq!(take_pending().map(|program| program.pid), Some(first));
        assert_eq!(take_pending().map(|program| program.pid), Some(second));
    }
}
