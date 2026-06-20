#![no_std]
#![no_main]

use core::panic::PanicInfo;

const IDLE_YIELD_SPINS: u32 = 64;
#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

#[derive(Clone, Copy)]
struct ShellState {
    history: [u8; 64],
    history_len: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    Char(u8),
    Backspace,
    Delete,
    Enter,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    ClearLine,
    Ignore,
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut state = ShellState {
        history: [0; 64],
        history_len: 0,
    };

    loop {
        write_prompt();

        let mut input = [0u8; 64];
        let len = read_line(&mut input, &state);
        write("\n");

        if !run_command(&input[..len]) {
            abi::exit(0);
        }
        save_history(&mut state, &input[..len]);
    }
}

fn read_line(input: &mut [u8], state: &ShellState) -> usize {
    let mut len = 0usize;
    let mut cursor = 0usize;
    let mut display_len = 0usize;
    loop {
        let old_cursor = cursor;
        let key = read_key();
        match key {
            Key::Enter => return len,
            Key::Backspace => backspace_at(input, &mut len, &mut cursor),
            Key::Delete => delete_at(input, &mut len, cursor),
            Key::Left => move_left(&mut cursor),
            Key::Right => move_right(len, &mut cursor),
            Key::Home => move_home(&mut cursor),
            Key::End => move_end(len, &mut cursor),
            Key::ClearLine => clear_line(input, &mut len, &mut cursor),
            Key::Up => recall_history(input, &mut len, &mut cursor, state),
            Key::Down => clear_line(input, &mut len, &mut cursor),
            Key::Char(ch) if (0x20..=0x7e).contains(&ch) => {
                insert_char(input, &mut len, &mut cursor, ch)
            }
            _ => {}
        }
        redraw_line(input, len, cursor, old_cursor, &mut display_len);
    }
}

fn read_key() -> Key {
    let first = read_byte();
    match first {
        b'\n' => Key::Enter,
        8 | 127 => Key::Backspace,
        21 => Key::ClearLine,
        27 => read_escape_key(),
        byte => Key::Char(byte),
    }
}

fn read_escape_key() -> Key {
    if read_byte() != b'[' {
        return Key::Ignore;
    }

    match read_byte() {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'1' | b'7' => {
            let _ = read_byte();
            Key::Home
        }
        b'3' => {
            let _ = read_byte();
            Key::Delete
        }
        b'4' | b'8' => {
            let _ = read_byte();
            Key::End
        }
        _ => Key::Ignore,
    }
}

fn read_byte() -> u8 {
    let mut idle_spins = 0u32;
    loop {
        let mut byte = [0u8; 1];
        let read = abi::read(abi::STDIN, &mut byte);
        if read == abi::ERR_WOULD_BLOCK || read == 0 {
            idle_spins += 1;
            if idle_spins >= IDLE_YIELD_SPINS {
                idle_spins = 0;
                abi::yield_now();
            }
            continue;
        } else if read < 0 {
            continue;
        }
        return byte[0];
    }
}

fn insert_char(input: &mut [u8], len: &mut usize, cursor: &mut usize, ch: u8) {
    if *len >= input.len() {
        return;
    }
    let mut index = *len;
    while index > *cursor {
        input[index] = input[index - 1];
        index -= 1;
    }
    input[*cursor] = ch;
    *len += 1;
    *cursor += 1;
}

fn backspace_at(input: &mut [u8], len: &mut usize, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    *cursor -= 1;
    let mut index = *cursor;
    while index + 1 < *len {
        input[index] = input[index + 1];
        index += 1;
    }
    *len -= 1;
}

fn delete_at(input: &mut [u8], len: &mut usize, cursor: usize) {
    if cursor >= *len {
        return;
    }
    let mut index = cursor;
    while index + 1 < *len {
        input[index] = input[index + 1];
        index += 1;
    }
    *len -= 1;
}

fn move_left(cursor: &mut usize) {
    if *cursor > 0 {
        *cursor -= 1;
    }
}

fn move_right(len: usize, cursor: &mut usize) {
    if *cursor < len {
        *cursor += 1;
    }
}

fn move_home(cursor: &mut usize) {
    *cursor = 0;
}

fn move_end(len: usize, cursor: &mut usize) {
    *cursor = len;
}

fn clear_line(input: &mut [u8], len: &mut usize, cursor: &mut usize) {
    input[..*len].fill(0);
    *len = 0;
    *cursor = 0;
}

fn recall_history(input: &mut [u8], len: &mut usize, cursor: &mut usize, state: &ShellState) {
    if state.history_len == 0 {
        return;
    }
    clear_line(input, len, cursor);
    input[..state.history_len].copy_from_slice(&state.history[..state.history_len]);
    *len = state.history_len;
    *cursor = state.history_len;
}

fn redraw_line(
    input: &[u8],
    len: usize,
    cursor: usize,
    old_cursor: usize,
    display_len: &mut usize,
) {
    move_cursor_left(old_cursor);
    write_bytes(&input[..len]);

    let old_len = *display_len;
    if old_len > len {
        for _ in len..old_len {
            write(" ");
        }
    }

    let rendered_len = old_len.max(len);
    move_cursor_left(rendered_len.saturating_sub(cursor));
    *display_len = len;
}

fn move_cursor_left(count: usize) {
    for _ in 0..count {
        write_bytes(&[8]);
    }
}

fn save_history(state: &mut ShellState, line: &[u8]) {
    let line = trim(line);
    if line.is_empty() {
        return;
    }
    let count = line.len().min(state.history.len());
    state.history[..count].copy_from_slice(&line[..count]);
    state.history_len = count;
}

fn run_command(line: &[u8]) -> bool {
    let line = trim(line);
    if line.is_empty() {
        return true;
    }

    let (cmd, arg) = split_once(line);
    if eq(cmd, b"exit") {
        return false;
    }

    if eq(cmd, b"reboot") {
        write("rebooting...\n");
        abi::reboot();
    }

    if eq(cmd, b"help") {
        write(
            "help bg cd pwd ls cat stat procs pci netdev dmesg drvstat kill sleep uptime whoami rusthello yielddemo preemptdemo threaddemo pipedemo eventdemo msgdemo signaldemo jobdemo reboot exit\n",
        );
        return true;
    }

    if eq(cmd, b"cd") {
        let path = arg.unwrap_or(b"/");
        if abi::chdir(path) < 0 {
            write("cd: failed\n");
        }
        return true;
    }

    if eq(cmd, b"pwd") {
        let mut buffer = [0u8; 128];
        let len = abi::getcwd(&mut buffer);
        if len < 0 {
            write("pwd: failed\n");
        } else {
            write_bytes(&buffer[..len as usize]);
            write("\n");
        }
        return true;
    }

    if eq(cmd, b"bg") {
        let Some(background) = arg else {
            write("usage: bg COMMAND\n");
            return true;
        };
        let (background_cmd, background_arg) = split_once(background);
        let result = exec_command_background(background_cmd, background_arg);
        if result < 0 {
            write("vsh: command not found\n");
        } else {
            write("bg: queued\n");
            abi::yield_now();
        }
        return true;
    }

    let result = exec_command(cmd, arg);

    if result < 0 {
        write("vsh: command not found\n");
        return true;
    } else {
        abi::waitpid(result as u64);
    }
    true
}

fn exec_command(cmd: &[u8], arg: Option<&[u8]>) -> i64 {
    exec_command_with_mode(cmd, arg, false)
}

fn exec_command_background(cmd: &[u8], arg: Option<&[u8]>) -> i64 {
    exec_command_with_mode(cmd, arg, true)
}

fn exec_command_with_mode(cmd: &[u8], arg: Option<&[u8]>, background: bool) -> i64 {
    if eq(cmd, b"ls") {
        exec_with_mode(cmd, Some(arg.unwrap_or(b".")), background)
    } else if eq(cmd, b"cat")
        || eq(cmd, b"stat")
        || eq(cmd, b"kill")
        || eq(cmd, b"fault")
        || eq(cmd, b"sleep")
        || eq(cmd, b"pwd")
    {
        exec_with_mode(cmd, arg, background)
    } else if eq(cmd, b"uptime")
        || eq(cmd, b"whoami")
        || eq(cmd, b"procs")
        || eq(cmd, b"pci")
        || eq(cmd, b"netdev")
        || eq(cmd, b"dmesg")
        || eq(cmd, b"drvstat")
        || eq(cmd, b"rusthello")
        || eq(cmd, b"yielddemo")
        || eq(cmd, b"preemptdemo")
        || eq(cmd, b"threaddemo")
        || eq(cmd, b"pipedemo")
        || eq(cmd, b"eventdemo")
        || eq(cmd, b"msgdemo")
        || eq(cmd, b"signaldemo")
        || eq(cmd, b"jobdemo")
        || eq(cmd, b"demo")
    {
        exec_with_mode(cmd, None, background)
    } else {
        exec_with_mode(cmd, arg, background)
    }
}

fn split_once(line: &[u8]) -> (&[u8], Option<&[u8]>) {
    let mut index = 0usize;
    while index < line.len() && line[index] != b' ' {
        index += 1;
    }

    let command = &line[..index];
    let arg = trim(&line[index..]);
    let arg = if arg.is_empty() { None } else { Some(arg) };
    (command, arg)
}

fn trim(bytes: &[u8]) -> &[u8] {
    let mut start = 0usize;
    let mut end = bytes.len();

    while start < end && bytes[start] == b' ' {
        start += 1;
    }
    while end > start && bytes[end - 1] == b' ' {
        end -= 1;
    }

    &bytes[start..end]
}

fn eq(left: &[u8], right: &[u8]) -> bool {
    left == right
}

fn exec_with_mode(path: &[u8], arg: Option<&[u8]>, background: bool) -> i64 {
    if background {
        abi::exec_background_bytes(path, arg)
    } else {
        abi::exec_bytes(path, arg)
    }
}

fn write_prompt() {
    let mut user = [0u8; 32];
    let user_len = abi::whoami(&mut user);
    if user_len == 0 {
        write("user");
    } else {
        write_bytes(&user[..user_len]);
    }

    write(":");

    let mut cwd = [0u8; 128];
    let cwd_len = abi::getcwd(&mut cwd);
    if cwd_len < 0 {
        write("?");
    } else {
        write_bytes(&cwd[..cwd_len as usize]);
    }

    write("$ ");
}

fn write(text: &str) {
    abi::write(text);
}

fn write_bytes(bytes: &[u8]) {
    abi::write_bytes(abi::STDOUT, bytes);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(1);
}
