use core::fmt;

use crate::vga_buffer::{self, Color};

pub fn clear() {
    vga_buffer::clear_screen();
}

pub fn set_color(foreground: Color, background: Color) {
    vga_buffer::set_color(foreground, background);
}

pub fn backspace() {
    vga_buffer::erase_previous();
}

pub fn write_at(row: usize, col: usize, byte: u8, foreground: Color, background: Color) {
    vga_buffer::write_at(row, col, byte, foreground, background);
}

pub fn status(row: usize, args: fmt::Arguments) {
    vga_buffer::write_status_line(row, args);
}
