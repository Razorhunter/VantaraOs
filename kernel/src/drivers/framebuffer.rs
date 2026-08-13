use crate::sync::PreemptMutex as Mutex;

pub const LEGACY_WIDTH: usize = 320;
pub const LEGACY_HEIGHT: usize = 200;
pub const LEGACY_STRIDE: usize = 320;
pub const LEGACY_BITS_PER_PIXEL: u8 = 8;
#[cfg(feature = "framebuffer-vga")]
const FRAMEBUFFER_ADDRESS: usize = 0x000a_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Indexed8,
    Rgb888,
    Bgr888,
    Xrgb8888,
    Bgrx8888,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramebufferInfo {
    pub ready: bool,
    pub address: usize,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub bits_per_pixel: u8,
    pub pixel_format: PixelFormat,
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
            pixel_format: PixelFormat::Indexed8,
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
        register(
            FRAMEBUFFER_ADDRESS,
            LEGACY_WIDTH,
            LEGACY_HEIGHT,
            LEGACY_STRIDE,
            8,
            PixelFormat::Indexed8,
        )
        .expect("legacy VGA framebuffer geometry must be valid");
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

pub fn init_boot_framebuffer(
    address: usize,
    width: usize,
    height: usize,
    stride: usize,
    bits_per_pixel: u8,
    pixel_format: PixelFormat,
) {
    match register(address, width, height, stride, bits_per_pixel, pixel_format) {
        Ok(()) => {
            clear(1);
            let checksum = framebuffer_checksum();
            INFO.lock().checksum = checksum;
            crate::drivers::status::report(
                "framebuffer",
                crate::drivers::status::DriverState::Ready,
                "bootloader linear framebuffer handoff",
            );
            crate::serial_println!(
                "[FB] boot-handoff address={:#x} width={} height={} stride={} bpp={} format={:?} checksum={:08x}",
                address,
                width,
                height,
                stride,
                bits_per_pixel,
                pixel_format,
                checksum
            );
        }
        Err(error) => {
            crate::drivers::status::report(
                "framebuffer",
                crate::drivers::status::DriverState::Error,
                "invalid bootloader framebuffer handoff",
            );
            crate::serial_println!("[FB] boot-handoff rejected: {:?}", error);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterError {
    NullAddress,
    UnsupportedBitsPerPixel,
    InvalidGeometry,
    SizeOverflow,
}

/// Registers a linear framebuffer supplied by the active boot/display backend.
///
/// `stride` is measured in bytes. The caller must ensure that `address` remains
/// mapped and writable for at least `stride * height` bytes while registered.
pub fn register(
    address: usize,
    width: usize,
    height: usize,
    stride: usize,
    bits_per_pixel: u8,
    pixel_format: PixelFormat,
) -> Result<(), RegisterError> {
    if address == 0 {
        return Err(RegisterError::NullAddress);
    }
    let expected_bits = match pixel_format {
        PixelFormat::Indexed8 => 8,
        PixelFormat::Rgb888 | PixelFormat::Bgr888 => 24,
        PixelFormat::Xrgb8888 | PixelFormat::Bgrx8888 => 32,
    };
    if bits_per_pixel != expected_bits {
        return Err(RegisterError::UnsupportedBitsPerPixel);
    }
    let bytes_per_pixel = usize::from(bits_per_pixel / 8);
    let minimum_stride = width
        .checked_mul(bytes_per_pixel)
        .ok_or(RegisterError::SizeOverflow)?;
    if width == 0 || height == 0 || stride < minimum_stride {
        return Err(RegisterError::InvalidGeometry);
    }
    stride
        .checked_mul(height)
        .ok_or(RegisterError::SizeOverflow)?;
    *INFO.lock() = FramebufferInfo {
        ready: true,
        address,
        width,
        height,
        stride,
        bits_per_pixel,
        pixel_format,
        writes: 0,
        clipped_pixels: 0,
        checksum: 0,
    };
    Ok(())
}

pub fn put_pixel(x: usize, y: usize, color: u8) -> bool {
    let mut info = INFO.lock();
    if !info.ready || x >= info.width || y >= info.height {
        info.clipped_pixels = info.clipped_pixels.saturating_add(1);
        return false;
    }
    let bytes_per_pixel = usize::from(info.bits_per_pixel / 8);
    let offset = y * info.stride + x * bytes_per_pixel;
    let [red, green, blue] = indexed_color(color);
    // SAFETY: registration requires the backend to keep `stride * height`
    // bytes mapped and writable; the bounds above constrain this byte write.
    let pixel = match info.pixel_format {
        PixelFormat::Indexed8 => [color, 0, 0, 0],
        PixelFormat::Rgb888 => [red, green, blue, 0],
        PixelFormat::Bgr888 => [blue, green, red, 0],
        PixelFormat::Xrgb8888 => [blue, green, red, 0],
        PixelFormat::Bgrx8888 => [red, green, blue, 0],
    };
    for (byte_index, byte) in pixel[..bytes_per_pixel].iter().copied().enumerate() {
        // SAFETY: the pixel offset and byte count are bounded by the registered
        // stride, geometry, and validated pixel size.
        unsafe {
            (info.address as *mut u8)
                .add(offset + byte_index)
                .write_volatile(byte)
        };
    }
    info.writes = info.writes.saturating_add(1);
    true
}

pub(crate) fn indexed_color(color: u8) -> [u8; 3] {
    const VGA_PALETTE: [[u8; 3]; 16] = [
        [0x00, 0x00, 0x00],
        [0x00, 0x00, 0xaa],
        [0x00, 0xaa, 0x00],
        [0x00, 0xaa, 0xaa],
        [0xaa, 0x00, 0x00],
        [0xaa, 0x00, 0xaa],
        [0xaa, 0x55, 0x00],
        [0xaa, 0xaa, 0xaa],
        [0x55, 0x55, 0x55],
        [0x55, 0x55, 0xff],
        [0x55, 0xff, 0x55],
        [0x55, 0xff, 0xff],
        [0xff, 0x55, 0x55],
        [0xff, 0x55, 0xff],
        [0xff, 0xff, 0x55],
        [0xff, 0xff, 0xff],
    ];
    VGA_PALETTE[usize::from(color & 0x0f)]
}

pub fn fill_rect(x: usize, y: usize, width: usize, height: usize, color: u8) {
    let info = info();
    let end_y = y.saturating_add(height).min(info.height);
    let end_x = x.saturating_add(width).min(info.width);
    for row in y.min(info.height)..end_y {
        for column in x.min(info.width)..end_x {
            put_pixel(column, row, color);
        }
    }
}

pub fn clear(color: u8) {
    let info = info();
    fill_rect(0, 0, info.width, info.height, color);
}

pub fn present(buffer: &[u8]) -> bool {
    let mut info = INFO.lock();
    let Some(framebuffer_size) = info.stride.checked_mul(info.height) else {
        return false;
    };
    if !info.ready || buffer.len() != framebuffer_size {
        return false;
    }
    let mut checksum = 0u32;
    for (index, byte) in buffer.iter().copied().enumerate() {
        // SAFETY: source length equals the registered framebuffer byte size.
        unsafe { (info.address as *mut u8).add(index).write_volatile(byte) };
        checksum = checksum.rotate_left(5).wrapping_add(u32::from(byte));
    }
    info.writes = info.writes.saturating_add(buffer.len() as u64);
    info.checksum = checksum;
    true
}

#[cfg(feature = "framebuffer-vga")]
fn draw_boot_test_pattern() {
    let info = info();
    clear(1);
    fill_rect(info.width / 2, 0, info.width / 2, info.height / 2, 2);
    fill_rect(0, info.height / 2, info.width / 2, info.height / 2, 4);
    fill_rect(
        info.width / 2,
        info.height / 2,
        info.width / 2,
        info.height / 2,
        14,
    );
    fill_rect(12, 12, info.width.saturating_sub(24), 4, 15);
    fill_rect(
        12,
        info.height.saturating_sub(16),
        info.width.saturating_sub(24),
        4,
        15,
    );
    let checksum = framebuffer_checksum();
    INFO.lock().checksum = checksum;
}

pub fn framebuffer_checksum() -> u32 {
    let info = *INFO.lock();
    if !info.ready {
        return 0;
    }
    let mut checksum = 0u32;
    let Some(framebuffer_size) = info.stride.checked_mul(info.height) else {
        return 0;
    };
    for index in 0..framebuffer_size {
        // SAFETY: index is bounded by the registered framebuffer byte size.
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
    use super::{LEGACY_HEIGHT, LEGACY_STRIDE, LEGACY_WIDTH};
    #[test_case]
    fn mode_13h_geometry_is_linear_and_bounded() {
        assert_eq!(LEGACY_WIDTH, LEGACY_STRIDE);
        assert_eq!(LEGACY_WIDTH * LEGACY_HEIGHT, 64_000);
    }
}
