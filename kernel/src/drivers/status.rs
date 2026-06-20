use crate::sync::PreemptMutex as Mutex;
use lazy_static::lazy_static;

const MAX_DRIVER_STATUSES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverState {
    Ready,
    Degraded,
    Missing,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DriverStatus {
    name: &'static str,
    state: DriverState,
    detail: &'static str,
}

lazy_static! {
    static ref STATUSES: Mutex<[Option<DriverStatus>; MAX_DRIVER_STATUSES]> =
        Mutex::new([None; MAX_DRIVER_STATUSES]);
}

pub fn report(name: &'static str, state: DriverState, detail: &'static str) {
    let mut statuses = STATUSES.lock();
    if let Some(status) = statuses
        .iter_mut()
        .flatten()
        .find(|status| status.name == name)
    {
        status.state = state;
        status.detail = detail;
        return;
    }

    let Some(slot) = statuses.iter_mut().find(|status| status.is_none()) else {
        return;
    };
    *slot = Some(DriverStatus {
        name,
        state,
        detail,
    });
}

pub fn write_to_buffer(out: &mut [u8]) -> usize {
    let statuses = STATUSES.lock();
    let mut writer = BufferWriter::new(out);
    writer.write_str("DRIVER       STATE      DETAIL\n");
    for status in statuses.iter().flatten() {
        writer.write_padded(status.name, 12);
        writer.write_padded(state_name(status.state), 11);
        writer.write_str(status.detail);
        writer.write_byte(b'\n');
    }
    writer.len()
}

fn state_name(state: DriverState) -> &'static str {
    match state {
        DriverState::Ready => "ready",
        DriverState::Degraded => "degraded",
        DriverState::Missing => "missing",
        DriverState::Error => "error",
    }
}

struct BufferWriter<'a> {
    out: &'a mut [u8],
    len: usize,
}

impl<'a> BufferWriter<'a> {
    fn new(out: &'a mut [u8]) -> Self {
        Self { out, len: 0 }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn write_byte(&mut self, byte: u8) {
        if self.len < self.out.len() {
            self.out[self.len] = byte;
            self.len += 1;
        }
    }

    fn write_str(&mut self, text: &str) {
        for byte in text.bytes() {
            self.write_byte(byte);
        }
    }

    fn write_padded(&mut self, text: &str, width: usize) {
        self.write_str(text);
        for _ in text.len()..width {
            self.write_byte(b' ');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DriverState, report, write_to_buffer};

    #[test_case]
    fn repeated_driver_report_updates_one_record() {
        report("testdrv", DriverState::Missing, "not found");
        report("testdrv", DriverState::Ready, "attached");

        let mut out = [0u8; 128];
        let len = write_to_buffer(&mut out);
        let text = core::str::from_utf8(&out[..len]).unwrap();
        assert!(text.contains("testdrv"));
        assert!(text.contains("ready"));
        assert!(!text.contains("not found"));
    }
}
