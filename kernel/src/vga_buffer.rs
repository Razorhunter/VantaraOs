use core::fmt;
use lazy_static::lazy_static;
use spin::Mutex;
use volatile::Volatile;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
struct ColorCode(u8);

impl ColorCode {
    fn new(foreground: Color, background: Color) -> ColorCode {
        ColorCode((background as u8) << 4 | (foreground as u8))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct ScreenChar {
    ascii_character: u8,
    color_code: ColorCode,
}

pub const BUFFER_HEIGHT: usize = 25;
pub const BUFFER_WIDTH: usize = 80;
const TEXT_CURSOR_BLINK_TICKS: u64 = crate::timer::TIMER_HZ as u64 / 2;

struct Buffer {
    chars: [[Volatile<ScreenChar>; BUFFER_WIDTH]; BUFFER_HEIGHT],
}

pub struct Writer {
    column_position: usize,
    color_code: ColorCode,
    buffer: &'static mut Buffer,
    mouse_visible: bool,
    mouse_row: usize,
    mouse_col: usize,
    original_char: u8,
    original_color_code: ColorCode,
    text_cursor_visible: bool,
    text_cursor_row: usize,
    text_cursor_col: usize,
    text_cursor_original_char: u8,
    text_cursor_original_color_code: ColorCode,
    text_cursor_blink_on: bool,
    text_cursor_last_blink_tick: u64,
}

impl Writer {
    pub fn write_byte(&mut self, byte: u8) {
        let mouse = self.suspend_mouse_cursor();
        self.hide_text_cursor();
        match byte {
            b'\n' => self.new_line(),
            8 => self.backspace(),
            byte => {
                if self.column_position >= BUFFER_WIDTH {
                    self.new_line();
                }

                let row = BUFFER_HEIGHT - 1;
                let col = self.column_position;

                let color_code = self.color_code;
                self.buffer.chars[row][col].write(ScreenChar {
                    ascii_character: byte,
                    color_code,
                });
                self.column_position += 1;
            }
        }
        self.reset_text_cursor_blink();
        self.show_text_cursor();
        self.resume_mouse_cursor(mouse);
    }

    pub fn write_string(&mut self, s: &str) {
        for byte in s.bytes() {
            match byte {
                // printable ASCII byte or newline
                0x20..=0x7e | b'\n' | 8 => self.write_byte(byte),
                // not part of printable ASCII range
                _ => self.write_byte(0xfe),
            }
        }
    }

    pub fn set_color(&mut self, foreground: Color, background: Color) {
        let mouse = self.suspend_mouse_cursor();
        self.hide_text_cursor();
        self.color_code = ColorCode::new(foreground, background);
        self.show_text_cursor();
        self.resume_mouse_cursor(mouse);
    }

    pub fn clear_screen(&mut self) {
        let mouse = self.suspend_mouse_cursor();
        self.hide_text_cursor();
        for row in 0..BUFFER_HEIGHT {
            self.clear_row(row);
        }
        self.column_position = 0;
        self.show_text_cursor();
        self.resume_mouse_cursor(mouse);
    }

    pub fn backspace(&mut self) {
        if self.column_position == 0 {
            return;
        }

        self.column_position -= 1;
    }

    pub fn erase_previous(&mut self) {
        let mouse = self.suspend_mouse_cursor();
        self.hide_text_cursor();
        self.backspace();
        let row = BUFFER_HEIGHT - 1;
        let col = self.column_position;
        self.buffer.chars[row][col].write(ScreenChar {
            ascii_character: b' ',
            color_code: self.color_code,
        });
        self.reset_text_cursor_blink();
        self.show_text_cursor();
        self.resume_mouse_cursor(mouse);
    }

    pub fn write_at(
        &mut self,
        row: usize,
        col: usize,
        byte: u8,
        foreground: Color,
        background: Color,
    ) {
        if row >= BUFFER_HEIGHT || col >= BUFFER_WIDTH {
            return;
        }

        let mouse = self.suspend_mouse_cursor();
        self.hide_text_cursor();
        let byte = match byte {
            0x20..=0x7e => byte,
            _ => 0xfe,
        };
        self.buffer.chars[row][col].write(ScreenChar {
            ascii_character: byte,
            color_code: ColorCode::new(foreground, background),
        });
        self.show_text_cursor();
        self.resume_mouse_cursor(mouse);
    }

    fn new_line(&mut self) {
        for row in 1..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                let character = self.buffer.chars[row][col].read();
                self.buffer.chars[row - 1][col].write(character);
            }
        }
        self.clear_row(BUFFER_HEIGHT - 1);
        self.column_position = 0;
    }

    fn clear_row(&mut self, row: usize) {
        let blank = ScreenChar {
            ascii_character: b' ',
            color_code: self.color_code,
        };
        for col in 0..BUFFER_WIDTH {
            self.buffer.chars[row][col].write(blank);
        }
    }

    pub fn write_status_line(&mut self, row: usize, args: fmt::Arguments) -> fmt::Result {
        if row >= BUFFER_HEIGHT {
            return Ok(());
        }

        let mouse = self.suspend_mouse_cursor();
        self.hide_text_cursor();
        let saved_column = self.column_position;
        self.clear_row(row);
        self.column_position = 0;

        let mut writer = LineWriter { writer: self, row };
        fmt::Write::write_fmt(&mut writer, args)?;

        self.column_position = saved_column;
        self.show_text_cursor();
        self.resume_mouse_cursor(mouse);
        Ok(())
    }

    pub fn draw_mouse_cursor(&mut self, col: usize, row: usize) {
        if col >= BUFFER_WIDTH || row >= BUFFER_HEIGHT {
            return;
        }

        // Hide previous cursor if visible
        if self.mouse_visible {
            let char = ScreenChar {
                ascii_character: self.original_char,
                color_code: self.original_color_code,
            };
            self.buffer.chars[self.mouse_row][self.mouse_col].write(char);
        }

        // Store original character and draw cursor
        let original = self.buffer.chars[row][col].read();
        self.original_char = original.ascii_character;
        self.original_color_code = original.color_code;

        let cursor_char = ScreenChar {
            ascii_character: b'*', // Mouse cursor symbol
            color_code: ColorCode::new(Color::White, Color::Red),
        };
        self.buffer.chars[row][col].write(cursor_char);

        self.mouse_visible = true;
        self.mouse_row = row;
        self.mouse_col = col;
    }

    pub fn clear_mouse_cursor(&mut self) {
        let _ = self.suspend_mouse_cursor();
    }

    fn suspend_mouse_cursor(&mut self) -> Option<(usize, usize)> {
        if !self.mouse_visible {
            return None;
        }

        let position = (self.mouse_col, self.mouse_row);
        self.buffer.chars[self.mouse_row][self.mouse_col].write(ScreenChar {
            ascii_character: self.original_char,
            color_code: self.original_color_code,
        });
        self.mouse_visible = false;
        Some(position)
    }

    fn resume_mouse_cursor(&mut self, position: Option<(usize, usize)>) {
        if let Some((col, row)) = position {
            self.draw_mouse_cursor(col, row);
        }
    }

    fn show_text_cursor(&mut self) {
        if !self.text_cursor_blink_on || self.text_cursor_visible {
            return;
        }

        let row = BUFFER_HEIGHT - 1;
        let col = self.column_position.min(BUFFER_WIDTH - 1);
        let original = self.buffer.chars[row][col].read();

        self.text_cursor_original_char = original.ascii_character;
        self.text_cursor_original_color_code = original.color_code;
        self.text_cursor_row = row;
        self.text_cursor_col = col;

        self.buffer.chars[row][col].write(ScreenChar {
            ascii_character: original.ascii_character,
            color_code: ColorCode::new(Color::Black, Color::Yellow),
        });
        self.text_cursor_visible = true;
    }

    fn hide_text_cursor(&mut self) {
        if !self.text_cursor_visible {
            return;
        }

        self.buffer.chars[self.text_cursor_row][self.text_cursor_col].write(ScreenChar {
            ascii_character: self.text_cursor_original_char,
            color_code: self.text_cursor_original_color_code,
        });
        self.text_cursor_visible = false;
    }

    fn reset_text_cursor_blink(&mut self) {
        self.text_cursor_blink_on = true;
        self.text_cursor_last_blink_tick = crate::timer::ticks();
    }

    pub fn update_text_cursor_blink(&mut self, ticks: u64) {
        if ticks.saturating_sub(self.text_cursor_last_blink_tick) < TEXT_CURSOR_BLINK_TICKS {
            return;
        }

        let mouse = self.suspend_mouse_cursor();
        self.text_cursor_last_blink_tick = ticks;
        if self.text_cursor_visible {
            self.hide_text_cursor();
            self.text_cursor_blink_on = false;
        } else {
            self.text_cursor_blink_on = true;
            self.show_text_cursor();
        }
        self.resume_mouse_cursor(mouse);
    }
}

struct LineWriter<'a> {
    writer: &'a mut Writer,
    row: usize,
}

impl fmt::Write for LineWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if self.writer.column_position >= BUFFER_WIDTH {
                break;
            }

            let byte = match byte {
                0x20..=0x7e => byte,
                _ => 0xfe,
            };
            let col = self.writer.column_position;
            self.writer.buffer.chars[self.row][col].write(ScreenChar {
                ascii_character: byte,
                color_code: ColorCode::new(Color::LightCyan, Color::Black),
            });
            self.writer.column_position += 1;
        }

        Ok(())
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_string(s);
        Ok(())
    }
}

lazy_static! {
    pub static ref WRITER: Mutex<Writer> = Mutex::new(Writer {
        column_position: 0,
        color_code: ColorCode::new(Color::Yellow, Color::Black),
        buffer: unsafe { &mut *(0xb8000 as *mut Buffer) },
        mouse_visible: false,
        mouse_row: 0,
        mouse_col: 0,
        original_char: b' ',
        original_color_code: ColorCode::new(Color::Yellow, Color::Black),
        text_cursor_visible: false,
        text_cursor_row: BUFFER_HEIGHT - 1,
        text_cursor_col: 0,
        text_cursor_original_char: b' ',
        text_cursor_original_color_code: ColorCode::new(Color::Yellow, Color::Black),
        text_cursor_blink_on: true,
        text_cursor_last_blink_tick: 0,
    });
}

#[macro_export]
macro_rules! print
{
    ($($arg:tt)*) => ($crate::vga_buffer::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println
{
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;

    crate::sync::without_interrupts(|| {
        WRITER.lock().write_fmt(args).unwrap();
    });
}

pub fn draw_mouse_cursor(col: usize, row: usize) {
    crate::sync::without_interrupts(|| {
        WRITER.lock().draw_mouse_cursor(col, row);
    });
}

pub fn clear_mouse_cursor() {
    crate::sync::without_interrupts(|| {
        WRITER.lock().clear_mouse_cursor();
    });
}

pub fn update_mouse_cursor(col: usize, row: usize) {
    crate::sync::without_interrupts(|| {
        WRITER.lock().draw_mouse_cursor(col, row);
    });
}

pub fn set_color(foreground: Color, background: Color) {
    crate::sync::without_interrupts(|| {
        WRITER.lock().set_color(foreground, background);
    });
}

pub fn clear_screen() {
    crate::sync::without_interrupts(|| {
        WRITER.lock().clear_screen();
    });
}

pub fn backspace() {
    crate::sync::without_interrupts(|| {
        WRITER.lock().backspace();
    });
}

pub fn erase_previous() {
    crate::sync::without_interrupts(|| {
        WRITER.lock().erase_previous();
    });
}

pub fn write_at(row: usize, col: usize, byte: u8, foreground: Color, background: Color) {
    crate::sync::without_interrupts(|| {
        WRITER
            .lock()
            .write_at(row, col, byte, foreground, background);
    });
}

pub fn write_status_line(row: usize, args: fmt::Arguments) {
    crate::sync::without_interrupts(|| {
        WRITER.lock().write_status_line(row, args).unwrap();
    });
}

pub fn update_text_cursor_blink(ticks: u64) {
    crate::sync::without_interrupts(|| {
        WRITER.lock().update_text_cursor_blink(ticks);
    });
}

#[test_case]
fn test_println_simple() {
    println!("test_println_simple output");
}

#[test_case]
fn test_println_many() {
    for _ in 0..200 {
        println!("test_println_many output");
    }
}

#[test_case]
fn test_println_output() {
    use core::fmt::Write;
    use x86_64::instructions::interrupts;

    let s = "Some test string that fits on a single line";
    interrupts::without_interrupts(|| {
        let mut writer = WRITER.lock();
        writeln!(writer, "\n{}", s).expect("writeln failed");
        for (i, c) in s.chars().enumerate() {
            let screen_char = writer.buffer.chars[BUFFER_HEIGHT - 2][i].read();
            assert_eq!(char::from(screen_char.ascii_character), c);
        }
    });
}

#[test_case]
fn mouse_overlay_does_not_restore_stale_character_after_write() {
    use x86_64::instructions::interrupts;

    interrupts::without_interrupts(|| {
        let mut writer = WRITER.lock();
        writer.clear_screen();
        let color_code = writer.color_code;
        writer.buffer.chars[10][10].write(ScreenChar {
            ascii_character: b'o',
            color_code,
        });
        writer.draw_mouse_cursor(10, 10);

        writer.write_at(10, 10, b'R', Color::Yellow, Color::Black);
        writer.draw_mouse_cursor(11, 10);

        let restored = writer.buffer.chars[10][10].read();
        assert_eq!(restored.ascii_character, b'R');
        assert_eq!(
            restored.color_code,
            ColorCode::new(Color::Yellow, Color::Black)
        );
        writer.clear_mouse_cursor();
    });
}
