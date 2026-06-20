use crate::sync::PreemptMutex as Mutex;
use lazy_static::lazy_static;
use uart_16550::SerialPort;

const UART_RING_ENABLED: bool = true;
const UART_RING_CAPACITY: usize = 8192;

lazy_static! {
    pub static ref SERIAL1: Mutex<SerialPort> = {
        let mut serial_port = unsafe { SerialPort::new(0x3F8) };
        serial_port.init();
        Mutex::new(serial_port)
    };
    static ref UART_RING: Mutex<UartRingBuffer> = Mutex::new(UartRingBuffer::new());
}

pub fn init() {
    lazy_static::initialize(&SERIAL1);
    if UART_RING_ENABLED {
        lazy_static::initialize(&UART_RING);
    }
}

#[doc(hidden)]
pub fn _print(args: ::core::fmt::Arguments) {
    use core::fmt::Write;

    crate::sync::without_interrupts(|| {
        SERIAL1
            .lock()
            .write_fmt(args)
            .expect("Printing to serial failed");
        if UART_RING_ENABLED {
            UART_RING
                .lock()
                .write_fmt(args)
                .expect("Buffering serial output failed");
        }
    });
}

pub fn copy_recent_log(out: &mut [u8]) -> usize {
    if !UART_RING_ENABLED || out.is_empty() {
        return 0;
    }
    UART_RING.lock().copy_recent(out)
}

struct UartRingBuffer {
    bytes: [u8; UART_RING_CAPACITY],
    write_index: usize,
    len: usize,
}

impl UartRingBuffer {
    const fn new() -> Self {
        Self {
            bytes: [0; UART_RING_CAPACITY],
            write_index: 0,
            len: 0,
        }
    }

    fn push(&mut self, byte: u8) {
        self.bytes[self.write_index] = byte;
        self.write_index = (self.write_index + 1) % UART_RING_CAPACITY;
        self.len = (self.len + 1).min(UART_RING_CAPACITY);
    }

    fn copy_recent(&self, out: &mut [u8]) -> usize {
        let count = self.len.min(out.len());
        let oldest = (self.write_index + UART_RING_CAPACITY - self.len) % UART_RING_CAPACITY;
        let skip = self.len - count;

        for (index, slot) in out[..count].iter_mut().enumerate() {
            let source = (oldest + skip + index) % UART_RING_CAPACITY;
            *slot = self.bytes[source];
        }
        count
    }
}

impl core::fmt::Write for UartRingBuffer {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        for byte in text.bytes() {
            self.push(byte);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::UartRingBuffer;
    use core::fmt::Write;

    #[test_case]
    fn uart_ring_returns_latest_bytes() {
        let mut ring = UartRingBuffer::new();
        ring.write_str("alpha-beta").unwrap();

        let mut out = [0u8; 4];
        let len = ring.copy_recent(&mut out);
        assert_eq!(&out[..len], b"beta");
    }
}

#[macro_export]
macro_rules! serial_print
{
    ($($arg:tt)*) =>
    {
        $crate::serial::_print(format_args!($($arg)*));
    };
}

#[macro_export]
macro_rules! serial_println
{
    () => ($crate::serial_print!("\n"));
    ($fmt:expr) => ($crate::serial_print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::serial_print!(
        concat!($fmt, "\n"), $($arg)*));
}

#[macro_export]
macro_rules! log_error {
    ($fmt:expr) => ($crate::serial_println!(concat!("[ERROR] ", $fmt)));
    ($fmt:expr, $($arg:tt)*) => ($crate::serial_println!(concat!("[ERROR] ", $fmt), $($arg)*));
}

#[macro_export]
macro_rules! log_warn {
    ($fmt:expr) => ($crate::serial_println!(concat!("[WARN ] ", $fmt)));
    ($fmt:expr, $($arg:tt)*) => ($crate::serial_println!(concat!("[WARN ] ", $fmt), $($arg)*));
}

#[macro_export]
macro_rules! log_info {
    ($fmt:expr) => ($crate::serial_println!(concat!("[INFO ] ", $fmt)));
    ($fmt:expr, $($arg:tt)*) => ($crate::serial_println!(concat!("[INFO ] ", $fmt), $($arg)*));
}

#[macro_export]
macro_rules! log_debug {
    ($fmt:expr) => ($crate::serial_println!(concat!("[DEBUG] ", $fmt)));
    ($fmt:expr, $($arg:tt)*) => ($crate::serial_println!(concat!("[DEBUG] ", $fmt), $($arg)*));
}
