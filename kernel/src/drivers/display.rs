use alloc::vec;
use alloc::vec::Vec;

use crate::sync::PreemptMutex as Mutex;

pub use super::framebuffer::PixelFormat;
use super::{framebuffer, virtio_gpu};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayBackend {
    None,
    LegacyVga,
    UefiGop,
    VirtioGpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayMode {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub bits_per_pixel: u8,
    pub refresh_millihertz: u32,
    pub pixel_format: PixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayInfo {
    pub ready: bool,
    pub backend: DisplayBackend,
    pub mode: DisplayMode,
    pub allocated_buffers: u64,
    pub page_flips: u64,
    pub rejected_flips: u64,
    pub last_checksum: u32,
    pub output_count: u8,
    pub edid_available: bool,
}

const EMPTY_MODE: DisplayMode = DisplayMode {
    width: 0,
    height: 0,
    stride: 0,
    bits_per_pixel: 0,
    refresh_millihertz: 0,
    pixel_format: PixelFormat::Indexed8,
};

static DISPLAY: Mutex<DisplayInfo> = Mutex::new(DisplayInfo {
    ready: false,
    backend: DisplayBackend::None,
    mode: EMPTY_MODE,
    allocated_buffers: 0,
    page_flips: 0,
    rejected_flips: 0,
    last_checksum: 0,
    output_count: 0,
    edid_available: false,
});

pub fn init() {
    let virtio = virtio_gpu::info();
    if virtio.scanout_configured {
        let mode = DisplayMode {
            width: virtio.primary_width as usize,
            height: virtio.primary_height as usize,
            stride: virtio.primary_width as usize * 4,
            bits_per_pixel: 32,
            refresh_millihertz: 60_000,
            pixel_format: PixelFormat::Xrgb8888,
        };
        *DISPLAY.lock() = DisplayInfo {
            ready: true,
            backend: DisplayBackend::VirtioGpu,
            mode,
            allocated_buffers: 0,
            page_flips: 0,
            rejected_flips: 0,
            last_checksum: 0,
            output_count: 1,
            edid_available: false,
        };
        crate::drivers::status::report(
            "display",
            crate::drivers::status::DriverState::Ready,
            "generic display API with VirtIO-GPU backend",
        );
        crate::serial_println!(
            "[DISPLAY] backend=VirtioGpu mode={}x{} stride={} bpp=32 refresh-millihertz=60000 outputs=1 output=virtio-primary edid=unavailable",
            mode.width,
            mode.height,
            mode.stride
        );
        return;
    }
    let framebuffer = framebuffer::info();
    if !framebuffer.ready {
        crate::drivers::status::report(
            "display",
            crate::drivers::status::DriverState::Missing,
            "no display backend registered",
        );
        return;
    }
    let mode = DisplayMode {
        width: framebuffer.width,
        height: framebuffer.height,
        stride: framebuffer.stride,
        bits_per_pixel: framebuffer.bits_per_pixel,
        refresh_millihertz: 70_000,
        pixel_format: framebuffer.pixel_format,
    };
    let backend = if cfg!(feature = "modern-boot") {
        DisplayBackend::UefiGop
    } else {
        DisplayBackend::LegacyVga
    };
    *DISPLAY.lock() = DisplayInfo {
        ready: true,
        backend,
        mode,
        allocated_buffers: 0,
        page_flips: 0,
        rejected_flips: 0,
        last_checksum: framebuffer.checksum,
        output_count: 1,
        edid_available: false,
    };
    crate::drivers::status::report(
        "display",
        crate::drivers::status::DriverState::Ready,
        if backend == DisplayBackend::UefiGop {
            "generic display API with UEFI GOP backend"
        } else {
            "generic display API with legacy VGA backend"
        },
    );
    crate::serial_println!(
        "[DISPLAY] backend={:?} mode={}x{} stride={} bpp={} refresh-millihertz={} outputs=1 output=firmware-primary edid=unavailable",
        backend,
        mode.width,
        mode.height,
        mode.stride,
        mode.bits_per_pixel,
        mode.refresh_millihertz
    );
}

pub fn info() -> DisplayInfo {
    *DISPLAY.lock()
}

pub fn create_scanout_buffer() -> Option<Vec<u8>> {
    let mut display = DISPLAY.lock();
    if !display.ready {
        return None;
    }
    let length = display.mode.stride.checked_mul(display.mode.height)?;
    display.allocated_buffers = display.allocated_buffers.saturating_add(1);
    Some(vec![0; length])
}

pub fn present(buffer: &[u8]) -> bool {
    let mode = info().mode;
    present_damage(
        buffer,
        DamageRect {
            x: 0,
            y: 0,
            width: mode.width,
            height: mode.height,
        },
    )
}

pub fn present_damage(buffer: &[u8], damage: DamageRect) -> bool {
    let (expected, backend, mode) = {
        let display = DISPLAY.lock();
        if !display.ready {
            return false;
        }
        (
            display.mode.stride.saturating_mul(display.mode.height),
            display.backend,
            display.mode,
        )
    };
    if buffer.len() != expected
        || damage.width == 0
        || damage.height == 0
        || damage.x.saturating_add(damage.width) > mode.width
        || damage.y.saturating_add(damage.height) > mode.height
    {
        let mut display = DISPLAY.lock();
        display.rejected_flips = display.rejected_flips.saturating_add(1);
        return false;
    }
    let presented = match backend {
        DisplayBackend::VirtioGpu => virtio_gpu::present_region(
            buffer,
            damage.x,
            damage.y,
            damage.width,
            damage.height,
            mode.stride,
        ),
        _ => framebuffer::present(buffer),
    };
    if !presented {
        let mut display = DISPLAY.lock();
        display.rejected_flips = display.rejected_flips.saturating_add(1);
        return false;
    }
    let mut display = DISPLAY.lock();
    let bytes_per_pixel = usize::from(mode.bits_per_pixel / 8);
    let row_start = damage.x * bytes_per_pixel;
    let row_bytes = damage.width * bytes_per_pixel;
    let mut checksum = display.last_checksum.rotate_left(7);
    for y in damage.y..damage.y + damage.height {
        for byte in &buffer[y * mode.stride + row_start..y * mode.stride + row_start + row_bytes] {
            checksum = checksum.rotate_left(5).wrapping_add(u32::from(*byte));
        }
    }
    display.page_flips = display.page_flips.saturating_add(1);
    display.last_checksum = checksum;
    true
}

pub fn write_to_buffer(out: &mut [u8]) -> usize {
    let display = *DISPLAY.lock();
    let mut writer = Writer { out, len: 0 };
    writer.text("ready=");
    writer.dec(display.ready as u64);
    writer.text(" backend=");
    writer.text(match display.backend {
        DisplayBackend::None => "none",
        DisplayBackend::LegacyVga => "legacy-vga",
        DisplayBackend::UefiGop => "uefi-gop",
        DisplayBackend::VirtioGpu => "virtio-gpu",
    });
    writer.text(" mode=");
    writer.dec(display.mode.width as u64);
    writer.byte(b'x');
    writer.dec(display.mode.height as u64);
    writer.text(" stride=");
    writer.dec(display.mode.stride as u64);
    writer.text(" bpp=");
    writer.dec(u64::from(display.mode.bits_per_pixel));
    writer.text(" buffers=");
    writer.dec(display.allocated_buffers);
    writer.text(" flips=");
    writer.dec(display.page_flips);
    writer.text(" rejected=");
    writer.dec(display.rejected_flips);
    writer.text(" checksum=0x");
    writer.hex(u64::from(display.last_checksum), 8);
    writer.text(" outputs=");
    writer.dec(u64::from(display.output_count));
    writer.text(" output=");
    writer.text(if display.output_count == 0 {
        "none"
    } else {
        match display.backend {
            DisplayBackend::VirtioGpu => "virtio-primary",
            _ => "firmware-primary",
        }
    });
    writer.text(" edid=");
    writer.text(if display.edid_available {
        "available"
    } else {
        "unavailable"
    });
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
    use super::{DisplayBackend, PixelFormat};
    #[test_case]
    fn backend_and_pixel_formats_are_explicit() {
        assert_ne!(DisplayBackend::LegacyVga, DisplayBackend::VirtioGpu);
        assert_ne!(PixelFormat::Indexed8, PixelFormat::Xrgb8888);
    }
}
