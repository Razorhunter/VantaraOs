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

    fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self
            .x
            .saturating_add(self.width)
            .max(other.x.saturating_add(other.width));
        let bottom = self
            .y
            .saturating_add(self.height)
            .max(other.y.saturating_add(other.height));
        Self {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    pub id: u32,
    pub owner_pid: u32,
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
    next_surface_id: u32,
    focused_surface: Option<u32>,
    protocol_requests: u64,
    rejected_requests: u64,
}

const MAX_SURFACES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceError {
    Unavailable,
    InvalidGeometry,
    LimitReached,
    NotFound,
    PermissionDenied,
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
        next_surface_id: 3,
        focused_surface: None,
        protocol_requests: 0,
        rejected_requests: 0,
    };
    compositor.add_surface(Surface {
        id: 1,
        owner_pid: 0,
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
        owner_pid: 0,
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

pub fn create_surface(
    owner_pid: u32,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    color: u8,
) -> Result<u32, SurfaceError> {
    with_compositor(|compositor| {
        if width == 0
            || height == 0
            || x >= compositor.width
            || y >= compositor.height
            || width > compositor.width
            || height > compositor.height
        {
            return Err(SurfaceError::InvalidGeometry);
        }
        if compositor.surfaces.len() >= MAX_SURFACES {
            return Err(SurfaceError::LimitReached);
        }
        let id = compositor.next_surface_id;
        compositor.next_surface_id = compositor.next_surface_id.saturating_add(1);
        let z = compositor
            .surfaces
            .iter()
            .map(|surface| surface.z)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let bounds = Rect {
            x,
            y,
            width,
            height,
        };
        compositor.add_surface(Surface {
            id,
            owner_pid,
            bounds,
            color: color & 0x0f,
            z,
            visible: true,
        });
        compositor.focused_surface = Some(id);
        compositor.redraw(bounds);
        Ok(id)
    })
}

pub fn configure_surface(
    owner_pid: u32,
    id: u32,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Result<(), SurfaceError> {
    with_compositor(|compositor| {
        if width == 0 || height == 0 || x >= compositor.width || y >= compositor.height {
            return Err(SurfaceError::InvalidGeometry);
        }
        let screen_width = compositor.width;
        let screen_height = compositor.height;
        let surface = owned_surface_mut(compositor, owner_pid, id)?;
        let old_bounds = surface.bounds;
        let new_bounds = Rect {
            x,
            y,
            width,
            height,
        }
        .clipped(screen_width, screen_height);
        surface.bounds = new_bounds;
        compositor.redraw(old_bounds.union(new_bounds));
        Ok(())
    })
}

pub fn set_surface_color(owner_pid: u32, id: u32, color: u8) -> Result<(), SurfaceError> {
    with_compositor(|compositor| {
        let surface = owned_surface_mut(compositor, owner_pid, id)?;
        surface.color = color & 0x0f;
        let bounds = surface.bounds;
        compositor.redraw(bounds);
        Ok(())
    })
}

pub fn focus_surface(owner_pid: u32, id: u32) -> Result<(), SurfaceError> {
    with_compositor(|compositor| {
        let next_z = compositor
            .surfaces
            .iter()
            .map(|surface| surface.z)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let surface = owned_surface_mut(compositor, owner_pid, id)?;
        surface.z = next_z;
        let bounds = surface.bounds;
        compositor.surfaces.sort_by_key(|surface| surface.z);
        compositor.focused_surface = Some(id);
        compositor.redraw(bounds);
        Ok(())
    })
}

pub fn destroy_surface(owner_pid: u32, id: u32) -> Result<(), SurfaceError> {
    with_compositor(|compositor| {
        let index = compositor
            .surfaces
            .iter()
            .position(|surface| surface.id == id)
            .ok_or(SurfaceError::NotFound)?;
        if compositor.surfaces[index].owner_pid != owner_pid {
            return Err(SurfaceError::PermissionDenied);
        }
        let bounds = compositor.surfaces.remove(index).bounds;
        if compositor.focused_surface == Some(id) {
            compositor.focused_surface = None;
        }
        compositor.redraw(bounds);
        Ok(())
    })
}

pub fn damage_surface(
    owner_pid: u32,
    id: u32,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Result<u64, SurfaceError> {
    with_compositor(|compositor| {
        if width == 0 || height == 0 {
            return Err(SurfaceError::InvalidGeometry);
        }
        let surface = owned_surface_mut(compositor, owner_pid, id)?;
        let local = Rect {
            x,
            y,
            width,
            height,
        }
        .clipped(surface.bounds.width, surface.bounds.height);
        if local.width == 0 || local.height == 0 {
            return Err(SurfaceError::InvalidGeometry);
        }
        let damage = Rect {
            x: surface.bounds.x.saturating_add(local.x),
            y: surface.bounds.y.saturating_add(local.y),
            width: local.width,
            height: local.height,
        };
        compositor.redraw(damage);
        Ok(compositor.frames)
    })
}

pub fn fence_completed(fence: u64) -> bool {
    COMPOSITOR
        .lock()
        .as_ref()
        .is_some_and(|compositor| fence != 0 && compositor.frames >= fence)
}

pub fn destroy_owned_surfaces(owner_pid: u32) -> usize {
    let mut state = COMPOSITOR.lock();
    let Some(compositor) = state.as_mut() else {
        return 0;
    };
    let before = compositor.surfaces.len();
    compositor
        .surfaces
        .retain(|surface| surface.owner_pid != owner_pid);
    let removed = before.saturating_sub(compositor.surfaces.len());
    if removed > 0 {
        if compositor
            .focused_surface
            .is_some_and(|id| !compositor.surfaces.iter().any(|surface| surface.id == id))
        {
            compositor.focused_surface = None;
        }
        compositor.redraw(Rect::full_screen(compositor.width, compositor.height));
    }
    removed
}

fn with_compositor<T>(
    operation: impl FnOnce(&mut Compositor) -> Result<T, SurfaceError>,
) -> Result<T, SurfaceError> {
    let mut state = COMPOSITOR.lock();
    let compositor = state.as_mut().ok_or(SurfaceError::Unavailable)?;
    compositor.protocol_requests = compositor.protocol_requests.saturating_add(1);
    let result = operation(compositor);
    if result.is_err() {
        compositor.rejected_requests = compositor.rejected_requests.saturating_add(1);
    }
    result
}

fn owned_surface_mut(
    compositor: &mut Compositor,
    owner_pid: u32,
    id: u32,
) -> Result<&mut Surface, SurfaceError> {
    let surface = compositor
        .surfaces
        .iter_mut()
        .find(|surface| surface.id == id)
        .ok_or(SurfaceError::NotFound)?;
    if surface.owner_pid != owner_pid {
        return Err(SurfaceError::PermissionDenied);
    }
    Ok(surface)
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
        display::present_damage(
            &self.backbuffer,
            display::DamageRect {
                x: damage.x,
                y: damage.y,
                width: damage.width,
                height: damage.height,
            },
        );
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
    writer.text(" focus=");
    writer.dec(u64::from(compositor.focused_surface.unwrap_or(0)));
    writer.text(" requests=");
    writer.dec(compositor.protocol_requests);
    writer.text(" rejected=");
    writer.dec(compositor.rejected_requests);
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
    fn unions_old_and_new_surface_bounds() {
        let union = Rect {
            x: 10,
            y: 20,
            width: 30,
            height: 40,
        }
        .union(Rect {
            x: 25,
            y: 10,
            width: 40,
            height: 20,
        });
        assert_eq!(
            union,
            Rect {
                x: 10,
                y: 10,
                width: 55,
                height: 50,
            }
        );
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
