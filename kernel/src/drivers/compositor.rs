use alloc::vec::Vec;

use crate::sync::PreemptMutex as Mutex;

use super::display;
use super::display::PixelFormat;
use super::framebuffer;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    const fn full_screen(width: usize, height: usize) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    fn clipped(self, screen_width: usize, screen_height: usize) -> Self {
        let x = self.x.min(screen_width);
        let y = self.y.min(screen_height);
        Self {
            x,
            y,
            width: self
                .x
                .saturating_add(self.width)
                .min(screen_width)
                .saturating_sub(x),
            height: self
                .y
                .saturating_add(self.height)
                .min(screen_height)
                .saturating_sub(y),
        }
    }

    fn contains(self, x: usize, y: usize) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    pub id: u32,
    pub bounds: Rect,
    pub color: u8,
    pub z: i16,
    pub visible: bool,
}

struct Compositor {
    surfaces: Vec<Surface>,
    backbuffer: Vec<u8>,
    width: usize,
    height: usize,
    stride: usize,
    pixel_format: PixelFormat,
    cursor_x: usize,
    cursor_y: usize,
    frames: u64,
    damaged_pixels: u64,
    checksum: u32,
}

static COMPOSITOR: Mutex<Option<Compositor>> = Mutex::new(None);

pub fn init() {
    let display_info = display::info();
    if !display_info.ready {
        crate::drivers::status::report(
            "compositor",
            crate::drivers::status::DriverState::Missing,
            "framebuffer unavailable",
        );
        return;
    }
    let mut compositor = Compositor {
        surfaces: Vec::new(),
        backbuffer: display::create_scanout_buffer().unwrap_or_default(),
        width: display_info.mode.width,
        height: display_info.mode.height,
        stride: display_info.mode.stride,
        pixel_format: display_info.mode.pixel_format,
        cursor_x: display_info.mode.width / 2,
        cursor_y: display_info.mode.height / 2,
        frames: 0,
        damaged_pixels: 0,
        checksum: 0,
    };
    compositor.add_surface(Surface {
        id: 1,
        bounds: Rect {
            x: 24,
            y: 24,
            width: 190,
            height: 118,
        },
        color: 9,
        z: 10,
        visible: true,
    });
    compositor.add_surface(Surface {
        id: 2,
        bounds: Rect {
            x: 112,
            y: 68,
            width: 176,
            height: 104,
        },
        color: 12,
        z: 20,
        visible: true,
    });
    compositor.redraw(Rect::full_screen(compositor.width, compositor.height));
    #[cfg(feature = "modern-boot")]
    compositor.redraw(Rect {
        x: compositor.width.saturating_sub(8),
        y: compositor.height.saturating_sub(8),
        width: 32,
        height: 32,
    });
    crate::drivers::status::report(
        "compositor",
        crate::drivers::status::DriverState::Ready,
        "surface, z-order, damage, and cursor prototype",
    );
    crate::serial_println!(
        "[COMPOSITOR] ready surfaces={} frames={} damaged={} checksum={:08x} buffers={} flips={} rejected={}",
        compositor.surfaces.len(),
        compositor.frames,
        compositor.damaged_pixels,
        compositor.checksum,
        display::info().allocated_buffers,
        display::info().page_flips,
        display::info().rejected_flips
    );
    *COMPOSITOR.lock() = Some(compositor);
}

impl Compositor {
    fn add_surface(&mut self, surface: Surface) {
        self.surfaces.push(surface);
        self.surfaces.sort_by_key(|surface| surface.z);
    }

    fn redraw(&mut self, damage: Rect) {
        let damage = damage.clipped(self.width, self.height);
        for y in damage.y..damage.y + damage.height {
            for x in damage.x..damage.x + damage.width {
                let mut color = if (x / 16 + y / 16) % 2 == 0 { 1 } else { 8 };
                for surface in &self.surfaces {
                    if surface.visible && surface.bounds.contains(x, y) {
                        color = surface.color;
                    }
                }
                if x >= self.cursor_x
                    && y >= self.cursor_y
                    && x - self.cursor_x <= y - self.cursor_y
                    && x - self.cursor_x < 12
                    && y - self.cursor_y < 16
                {
                    color = 15;
                }
                write_pixel(
                    &mut self.backbuffer,
                    self.stride,
                    self.pixel_format,
                    x,
                    y,
                    color,
                );
            }
        }
        self.damaged_pixels = self
            .damaged_pixels
            .saturating_add((damage.width * damage.height) as u64);
        self.frames = self.frames.saturating_add(1);
        self.checksum = checksum(&self.backbuffer);
        display::present(&self.backbuffer);
    }
}

fn write_pixel(
    buffer: &mut [u8],
    stride: usize,
    format: PixelFormat,
    x: usize,
    y: usize,
    color: u8,
) {
    let [red, green, blue] = framebuffer::indexed_color(color);
    let (bytes_per_pixel, pixel) = match format {
        PixelFormat::Indexed8 => (1, [color, 0, 0, 0]),
        PixelFormat::Rgb888 => (3, [red, green, blue, 0]),
        PixelFormat::Bgr888 => (3, [blue, green, red, 0]),
        PixelFormat::Xrgb8888 => (4, [blue, green, red, 0]),
        PixelFormat::Bgrx8888 => (4, [red, green, blue, 0]),
    };
    let Some(offset) = y.checked_mul(stride).and_then(|row| {
        x.checked_mul(bytes_per_pixel)
            .and_then(|x| row.checked_add(x))
    }) else {
        return;
    };
    let Some(destination) = buffer.get_mut(offset..offset.saturating_add(bytes_per_pixel)) else {
        return;
    };
    destination.copy_from_slice(&pixel[..bytes_per_pixel]);
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0u32, |sum, byte| {
        sum.rotate_left(5).wrapping_add(u32::from(*byte))
    })
}

pub fn write_to_buffer(out: &mut [u8]) -> usize {
    let compositor = COMPOSITOR.lock();
    let Some(compositor) = compositor.as_ref() else {
        let text = b"ready=0\n";
        let length = text.len().min(out.len());
        out[..length].copy_from_slice(&text[..length]);
        return length;
    };
    let mut writer = Writer { out, len: 0 };
    writer.text("ready=1 surfaces=");
    writer.dec(compositor.surfaces.len() as u64);
    writer.text(" frames=");
    writer.dec(compositor.frames);
    writer.text(" damaged=");
    writer.dec(compositor.damaged_pixels);
    writer.text(" cursor=");
    writer.dec(compositor.cursor_x as u64);
    writer.byte(b',');
    writer.dec(compositor.cursor_y as u64);
    writer.text(" checksum=0x");
    writer.hex(u64::from(compositor.checksum), 8);
    writer.byte(b'\n');
    writer.len
}

struct Writer<'a> {
    out: &'a mut [u8],
    len: usize,
}
impl Writer<'_> {
    fn byte(&mut self, byte: u8) {
        if self.len < self.out.len() {
            self.out[self.len] = byte;
            self.len += 1;
        }
    }
    fn text(&mut self, text: &str) {
        for byte in text.bytes() {
            self.byte(byte);
        }
    }
    fn dec(&mut self, mut value: u64) {
        let mut digits = [0u8; 20];
        let mut start = 20;
        loop {
            start -= 1;
            digits[start] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        for byte in &digits[start..] {
            self.byte(*byte);
        }
    }
    fn hex(&mut self, value: u64, digits: usize) {
        for shift in (0..digits).rev() {
            let nibble = ((value >> (shift * 4)) & 15) as u8;
            self.byte(if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PixelFormat, Rect, write_pixel};
    #[test_case]
    fn clips_damage_to_screen() {
        let clipped = Rect {
            x: 310,
            y: 190,
            width: 40,
            height: 30,
        }
        .clipped(320, 200);
        assert_eq!(clipped.width, 10);
        assert_eq!(clipped.height, 10);
    }

    #[test_case]
    fn encodes_runtime_pixel_formats_with_stride() {
        let mut indexed = [0u8; 8];
        write_pixel(&mut indexed, 4, PixelFormat::Indexed8, 2, 1, 12);
        assert_eq!(indexed[6], 12);

        let mut xrgb = [0u8; 32];
        write_pixel(&mut xrgb, 16, PixelFormat::Xrgb8888, 1, 1, 12);
        assert_eq!(&xrgb[20..24], &[0x55, 0x55, 0xff, 0x00]);

        let mut bgrx = [0u8; 32];
        write_pixel(&mut bgrx, 16, PixelFormat::Bgrx8888, 1, 1, 12);
        assert_eq!(&bgrx[20..24], &[0xff, 0x55, 0x55, 0x00]);
    }
}
