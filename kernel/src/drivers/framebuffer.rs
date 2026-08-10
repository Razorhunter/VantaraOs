use crate::sync::PreemptMutex as Mutex;

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 200;
pub const STRIDE: usize = 320;
pub const BITS_PER_PIXEL: u8 = 8;
#[cfg(feature = "framebuffer-vga")]
const FRAMEBUFFER_ADDRESS: usize = 0x000a_0000;
const FRAMEBUFFER_SIZE: usize = STRIDE * HEIGHT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramebufferInfo {
    pub ready: bool,
    pub address: usize,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub bits_per_pixel: u8,
    pub writes: u64,
    pub clipped_pixels: u64,
    pub checksum: u32,
}

impl FramebufferInfo {
    const fn unavailable() -> Self {
        Self {
            ready: false,
            address: 0,
            width: 0,
            height: 0,
            stride: 0,
            bits_per_pixel: 0,
            writes: 0,
            clipped_pixels: 0,
            checksum: 0,
        }
    }
}

static INFO: Mutex<FramebufferInfo> = Mutex::new(FramebufferInfo::unavailable());

pub fn info() -> FramebufferInfo {
    *INFO.lock()
}

pub fn init() {
    #[cfg(feature = "framebuffer-vga")]
    {
        *INFO.lock() = FramebufferInfo {
            ready: true,
            address: FRAMEBUFFER_ADDRESS,
            width: WIDTH,
            height: HEIGHT,
            stride: STRIDE,
            bits_per_pixel: BITS_PER_PIXEL,
            writes: 0,
            clipped_pixels: 0,
            checksum: 0,
        };
        draw_boot_test_pattern();
        let info = *INFO.lock();
        crate::drivers::status::report(
            "framebuffer",
            crate::drivers::status::DriverState::Ready,
            "VGA mode 13h linear framebuffer",
        );
        crate::serial_println!(
            "[FB] ready address={:#x} width={} height={} stride={} bpp={} checksum={:08x}",
            info.address,
            info.width,
            info.height,
            info.stride,
            info.bits_per_pixel,
            info.checksum
        );
    }
    #[cfg(not(feature = "framebuffer-vga"))]
    crate::drivers::status::report(
        "framebuffer",
        crate::drivers::status::DriverState::Missing,
        "build without framebuffer-vga feature",
    );
}

pub fn put_pixel(x: usize, y: usize, color: u8) -> bool {
    let mut info = INFO.lock();
    if !info.ready || x >= info.width || y >= info.height {
        info.clipped_pixels = info.clipped_pixels.saturating_add(1);
        return false;
    }
    let offset = y * info.stride + x;
    // SAFETY: mode 13h maps the complete 64,000-byte linear framebuffer at
    // 0xA0000; bounds above constrain the volatile byte write to that mapping.
    unsafe { (info.address as *mut u8).add(offset).write_volatile(color) };
    info.writes = info.writes.saturating_add(1);
    true
}

pub fn fill_rect(x: usize, y: usize, width: usize, height: usize, color: u8) {
    let end_y = y.saturating_add(height).min(HEIGHT);
    let end_x = x.saturating_add(width).min(WIDTH);
    for row in y.min(HEIGHT)..end_y {
        for column in x.min(WIDTH)..end_x {
            put_pixel(column, row, color);
        }
    }
}

pub fn clear(color: u8) {
    fill_rect(0, 0, WIDTH, HEIGHT, color);
}

pub fn present(buffer: &[u8]) -> bool {
    if buffer.len() != FRAMEBUFFER_SIZE {
        return false;
    }
    let mut info = INFO.lock();
    if !info.ready {
        return false;
    }
    let mut checksum = 0u32;
    for (index, byte) in buffer.iter().copied().enumerate() {
        // SAFETY: source length equals the complete mapped framebuffer size.
        unsafe { (info.address as *mut u8).add(index).write_volatile(byte) };
        checksum = checksum.rotate_left(5).wrapping_add(u32::from(byte));
    }
    info.writes = info.writes.saturating_add(buffer.len() as u64);
    info.checksum = checksum;
    true
}

#[cfg(feature = "framebuffer-vga")]
fn draw_boot_test_pattern() {
    clear(1);
    fill_rect(WIDTH / 2, 0, WIDTH / 2, HEIGHT / 2, 2);
    fill_rect(0, HEIGHT / 2, WIDTH / 2, HEIGHT / 2, 4);
    fill_rect(WIDTH / 2, HEIGHT / 2, WIDTH / 2, HEIGHT / 2, 14);
    fill_rect(12, 12, WIDTH - 24, 4, 15);
    fill_rect(12, HEIGHT - 16, WIDTH - 24, 4, 15);
    let checksum = framebuffer_checksum();
    INFO.lock().checksum = checksum;
}

pub fn framebuffer_checksum() -> u32 {
    let info = *INFO.lock();
    if !info.ready {
        return 0;
    }
    let mut checksum = 0u32;
    for index in 0..FRAMEBUFFER_SIZE {
        // SAFETY: index iterates exactly over the mapped mode 13h buffer.
        let byte = unsafe { (info.address as *const u8).add(index).read_volatile() };
        checksum = checksum.rotate_left(5).wrapping_add(u32::from(byte));
    }
    checksum
}

pub fn write_to_buffer(out: &mut [u8]) -> usize {
    let info = *INFO.lock();
    let mut writer = Writer { out, len: 0 };
    writer.text("ready=");
    writer.dec(info.ready as u64);
    writer.text(" address=0x");
    writer.hex(info.address as u64, 16);
    writer.text(" width=");
    writer.dec(info.width as u64);
    writer.text(" height=");
    writer.dec(info.height as u64);
    writer.text(" stride=");
    writer.dec(info.stride as u64);
    writer.text(" bpp=");
    writer.dec(u64::from(info.bits_per_pixel));
    writer.text(" writes=");
    writer.dec(info.writes);
    writer.text(" clipped=");
    writer.dec(info.clipped_pixels);
    writer.text(" checksum=0x");
    writer.hex(u64::from(info.checksum), 8);
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
    use super::{HEIGHT, STRIDE, WIDTH};
    #[test_case]
    fn mode_13h_geometry_is_linear_and_bounded() {
        assert_eq!(WIDTH, STRIDE);
        assert_eq!(WIDTH * HEIGHT, 64_000);
    }
}
