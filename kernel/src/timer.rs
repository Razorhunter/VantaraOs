use core::sync::atomic::{AtomicU64, Ordering};

use x86_64::instructions::port::Port;

const PIT_COMMAND_PORT: u16 = 0x43;
const PIT_CHANNEL_0_PORT: u16 = 0x40;
const PIT_BASE_FREQUENCY_HZ: u32 = 1_193_182;

pub const TIMER_HZ: u32 = 100;

static TICKS: AtomicU64 = AtomicU64::new(0);

pub fn init() {
    set_pit_frequency(TIMER_HZ);
}

fn set_pit_frequency(frequency_hz: u32) {
    let divisor = (PIT_BASE_FREQUENCY_HZ / frequency_hz) as u16;

    unsafe {
        let mut command = Port::<u8>::new(PIT_COMMAND_PORT);
        let mut channel_0 = Port::<u8>::new(PIT_CHANNEL_0_PORT);

        command.write(0x36);
        channel_0.write((divisor & 0xff) as u8);
        channel_0.write((divisor >> 8) as u8);
    }
}

pub fn tick() {
    let ticks = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    crate::scheduler::SCHEDULER.tick();
    crate::vga_buffer::update_text_cursor_blink(ticks);
}

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub fn uptime_ms() -> u64 {
    ticks() * 1_000 / TIMER_HZ as u64
}

pub fn sleep_ticks(ticks_to_sleep: u64) {
    let wake_at = ticks().saturating_add(ticks_to_sleep);

    while ticks() < wake_at {
        x86_64::instructions::hlt();
    }
}

pub fn sleep_ms(milliseconds: u64) {
    let ticks_to_sleep = milliseconds.saturating_mul(TIMER_HZ as u64).div_ceil(1_000);
    sleep_ticks(ticks_to_sleep.max(1));
}
