use crate::sync::PreemptMutex as Mutex;
use lazy_static::lazy_static;
use pc_keyboard::KeyCode;

use crate::input::InputEvent;
use crate::{allocator, console, fs, power, print, println, scheduler};

const COMMAND_BUFFER_SIZE: usize = 128;

pub struct Shell {
    buffer: [u8; COMMAND_BUFFER_SIZE],
    len: usize,
}

impl Shell {
    pub const fn new() -> Self {
        Self {
            buffer: [0; COMMAND_BUFFER_SIZE],
            len: 0,
        }
    }

    pub fn init(&mut self) {
        println!("");
        println!("Vantara shell ready. Type `help`.");
        self.prompt();
    }

    pub fn handle_event(&mut self, event: &InputEvent) {
        match event {
            InputEvent::KeyboardChar(ch) => self.push_char(*ch),
            InputEvent::KeyboardKey { key, pressed } if *pressed => self.handle_key(*key),
            _ => {}
        }
    }

    fn handle_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Backspace => self.backspace(),
            KeyCode::Return | KeyCode::NumpadEnter => self.execute_current(),
            _ => {}
        }
    }

    fn push_char(&mut self, ch: char) {
        if !ch.is_ascii() || ch.is_ascii_control() {
            return;
        }

        if self.len >= COMMAND_BUFFER_SIZE - 1 {
            return;
        }

        self.buffer[self.len] = ch as u8;
        self.len += 1;
        print!("{}", ch);
    }

    fn backspace(&mut self) {
        if self.len == 0 {
            return;
        }

        self.len -= 1;
        console::backspace();
    }

    fn execute_current(&mut self) {
        println!("");

        let command = core::str::from_utf8(&self.buffer[..self.len])
            .unwrap_or("")
            .trim();
        let prompt = execute_command(command);

        self.len = 0;
        self.buffer.fill(0);

        if prompt {
            self.prompt();
        }
    }

    fn prompt(&self) {
        print!("vantara> ");
    }
}

fn execute_command(command: &str) -> bool {
    let mut parts = command.split_whitespace();
    let name = parts.next().unwrap_or("");

    match name {
        "" => {}
        "help" => {
            println!(
                "commands: help clear mem tasks procs pci bin bootinit run exec reboot stat + /bin commands"
            );
        }
        "clear" => {
            console::clear();
        }
        "mem" => {
            let heap = allocator::heap_stats();
            println!(
                "heap start={:#x} end={:#x} used={} free={} failures={}",
                heap.start, heap.end, heap.used_bytes, heap.free_bytes, heap.allocation_failures
            );
        }
        "tasks" => {
            let stats = scheduler::SCHEDULER.stats();
            println!(
                "tasks threads={} ready={} current_tid={:?} idle_tid={:?} switches={} ticks={}",
                stats.total_tasks,
                stats.ready_tasks,
                stats.current_task,
                stats.idle_thread,
                stats.total_context_switches,
                stats.total_ticks
            );
        }
        "procs" => {
            crate::user::process::print_processes();
        }
        "pci" => {
            println!("pci devices={}", crate::drivers::pci::device_count());
        }
        "run" => match parts.next() {
            Some(path) => {
                request_user_program_arg(path, parts.next());
                return false;
            }
            None => println!("usage: run <path>"),
        },
        "bin" => match list_directory("/bin") {
            Ok(()) => {}
            Err(_) => println!("bin: /bin: not found"),
        },
        "bootinit" => {
            request_user_program_arg("init", None);
            return false;
        }
        "exec" => match parts.next() {
            Some(path) => {
                request_user_program_arg(path, parts.next());
                return false;
            }
            None => println!("usage: exec <path>"),
        },
        "reboot" => {
            println!("rebooting...");
            power::reboot();
        }
        "stat" => match parts.next() {
            Some(path) => match fs::stat(path) {
                Ok(stat) => println!(
                    "{} size={} readonly={}",
                    stat.path, stat.size, stat.readonly as u8
                ),
                Err(_) => println!("stat: {}: not found", path),
            },
            None => println!("usage: stat <path>"),
        },
        _ => {
            if crate::user::images::normalize_path(name).is_some() {
                request_user_program_arg(name, parts.next());
                return false;
            }

            println!("unknown command: {}", command);
        }
    }

    true
}

fn list_directory(path: &str) -> Result<(), fs::FsError> {
    let mut buffer = [0u8; 2048];
    let len = fs::list_to_buffer("/", path, &mut buffer)?;
    let text = core::str::from_utf8(&buffer[..len]).map_err(|_| fs::FsError::InvalidPath)?;
    for entry in text.lines() {
        println!("{}/{}", path.trim_end_matches('/'), entry);
    }
    Ok(())
}

fn request_user_program_arg(path: &str, arg: Option<&str>) {
    match crate::user::program::request_path_with_arg(path, arg) {
        Ok(pid) => {
            println!("switching to user program {} pid={}...", path, pid);
        }
        Err(_) => {
            println!("run: {}: not found", path);
            prompt_after_failed_user_program_request();
        }
    }
}

fn prompt_after_failed_user_program_request() {
    prompt();
}

lazy_static! {
    pub static ref SHELL: Mutex<Shell> = Mutex::new(Shell::new());
}

pub fn init() {
    SHELL.lock().init();
}

pub fn handle_event(event: &InputEvent) {
    SHELL.lock().handle_event(event);
}

pub fn prompt() {
    print!("vantara> ");
}
