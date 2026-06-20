use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::sync::PreemptMutex as Mutex;
use lazy_static::lazy_static;

use pc_keyboard::KeyCode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    KeyboardKey { key: KeyCode, pressed: bool },
    KeyboardChar(char),
    MouseMove { x_delta: i16, y_delta: i16 },
    MouseButton { button: MouseButton, pressed: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModifierState {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_key: bool,
    pub fn_key: bool,
}

impl ModifierState {
    pub const fn new() -> Self {
        Self {
            shift: false,
            ctrl: false,
            alt: false,
            super_key: false,
            fn_key: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MousePosition {
    pub x: i32,
    pub y: i32,
}

impl MousePosition {
    pub fn new(x: i32, y: i32) -> Self {
        MousePosition { x, y }
    }

    pub fn clamp(&self, max_x: i32, max_y: i32) -> Self {
        MousePosition {
            x: self.x.max(0).min(max_x - 1),
            y: self.y.max(0).min(max_y - 1),
        }
    }
}

const EVENT_QUEUE_CAPACITY: usize = 256;
const KEYBOARD_BYTE_QUEUE_CAPACITY: usize = 512;

static KEYBOARD_EVENTS: AtomicU64 = AtomicU64::new(0);
static TYPED_CHAR_EVENTS: AtomicU64 = AtomicU64::new(0);
static MOUSE_MOVE_EVENTS: AtomicU64 = AtomicU64::new(0);
static MOUSE_BUTTON_EVENTS: AtomicU64 = AtomicU64::new(0);
static DROPPED_EVENTS: AtomicU64 = AtomicU64::new(0);
static TERMINAL_SIGNAL: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputStats {
    pub queue_capacity: usize,
    pub queued_events: usize,
    pub keyboard_events: u64,
    pub typed_char_events: u64,
    pub mouse_move_events: u64,
    pub mouse_button_events: u64,
    pub dropped_events: u64,
    pub modifiers: ModifierState,
}

pub struct InputQueue {
    events: VecDeque<InputEvent>,
    keyboard_bytes: VecDeque<u8>,
    mouse_pos: MousePosition,
    modifiers: ModifierState,
    left_button: bool,
    right_button: bool,
    middle_button: bool,
}

impl InputQueue {
    pub fn new() -> Self {
        InputQueue {
            events: VecDeque::with_capacity(EVENT_QUEUE_CAPACITY),
            keyboard_bytes: VecDeque::with_capacity(KEYBOARD_BYTE_QUEUE_CAPACITY),
            mouse_pos: MousePosition::new(40, 12), // center
            modifiers: ModifierState::new(),
            left_button: false,
            right_button: false,
            middle_button: false,
        }
    }

    pub fn enqueue(&mut self, event: InputEvent) {
        if self.events.len() < EVENT_QUEUE_CAPACITY {
            self.events.push_back(event);
        } else {
            DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn dequeue(&mut self) -> Option<InputEvent> {
        self.events.pop_front()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn handle_mouse_move(&mut self, x_delta: i16, y_delta: i16) {
        self.mouse_pos.x = (self.mouse_pos.x as i16 + x_delta) as i32;
        self.mouse_pos.y = (self.mouse_pos.y as i16 - y_delta) as i32; // Y inverted

        self.mouse_pos = self.mouse_pos.clamp(80, 25); // VGA buffer dimensions

        MOUSE_MOVE_EVENTS.fetch_add(1, Ordering::Relaxed);
        self.enqueue(InputEvent::MouseMove { x_delta, y_delta });
    }

    pub fn handle_mouse_button(&mut self, button: MouseButton, pressed: bool) {
        match button {
            MouseButton::Left => self.left_button = pressed,
            MouseButton::Right => self.right_button = pressed,
            MouseButton::Middle => self.middle_button = pressed,
        }

        MOUSE_BUTTON_EVENTS.fetch_add(1, Ordering::Relaxed);
        self.enqueue(InputEvent::MouseButton { button, pressed });
    }

    pub fn handle_keyboard_key(&mut self, key: KeyCode, pressed: bool) -> bool {
        self.update_modifier_state(key, pressed);
        KEYBOARD_EVENTS.fetch_add(1, Ordering::Relaxed);
        let terminal_control =
            pressed && self.modifiers.ctrl && matches!(key, KeyCode::C | KeyCode::Z);
        if terminal_control {
            let signal = match key {
                KeyCode::C => crate::user::process::SIGINT,
                KeyCode::Z => crate::user::process::SIGTSTP,
                _ => unreachable!(),
            };
            TERMINAL_SIGNAL.store(signal, Ordering::Release);
        }
        if pressed {
            if !terminal_control {
                self.enqueue_terminal_sequence(key);
            }
        }
        self.enqueue(InputEvent::KeyboardKey { key, pressed });
        terminal_control
    }

    pub fn handle_keyboard_char(&mut self, ch: char) {
        KEYBOARD_EVENTS.fetch_add(1, Ordering::Relaxed);
        TYPED_CHAR_EVENTS.fetch_add(1, Ordering::Relaxed);
        if ch.is_ascii() {
            self.enqueue_keyboard_byte(ch as u8);
        }
        self.enqueue(InputEvent::KeyboardChar(ch));
    }

    pub fn dequeue_keyboard_byte(&mut self) -> Option<u8> {
        self.keyboard_bytes.pop_front()
    }

    fn enqueue_terminal_sequence(&mut self, key: KeyCode) {
        if let Some(bytes) = terminal_sequence_for_key(key) {
            self.enqueue_keyboard_bytes(bytes);
        }
    }

    fn enqueue_keyboard_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.enqueue_keyboard_byte(*byte);
        }
    }

    fn enqueue_keyboard_byte(&mut self, byte: u8) {
        if self.keyboard_bytes.len() < KEYBOARD_BYTE_QUEUE_CAPACITY {
            self.keyboard_bytes.push_back(byte);
        } else {
            DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn update_modifier_state(&mut self, key: KeyCode, pressed: bool) {
        match key {
            KeyCode::LShift | KeyCode::RShift => self.modifiers.shift = pressed,
            KeyCode::LControl | KeyCode::RControl => self.modifiers.ctrl = pressed,
            KeyCode::LAlt | KeyCode::RAltGr => self.modifiers.alt = pressed,
            KeyCode::LWin | KeyCode::RWin => self.modifiers.super_key = pressed,
            _ => {}
        }
    }

    pub fn modifiers(&self) -> ModifierState {
        self.modifiers
    }

    pub fn mouse_position(&self) -> MousePosition {
        self.mouse_pos
    }

    pub fn mouse_left_pressed(&self) -> bool {
        self.left_button
    }

    pub fn mouse_right_pressed(&self) -> bool {
        self.right_button
    }

    pub fn mouse_middle_pressed(&self) -> bool {
        self.middle_button
    }
}

lazy_static! {
    pub static ref INPUT_QUEUE: Mutex<InputQueue> = Mutex::new(InputQueue::new());
}

pub fn enqueue_event(event: InputEvent) {
    INPUT_QUEUE.lock().enqueue(event);
}

pub fn dequeue_event() -> Option<InputEvent> {
    INPUT_QUEUE.lock().dequeue()
}

pub fn try_dequeue_event() -> Option<InputEvent> {
    dequeue_event()
}

pub fn dequeue_keyboard_byte() -> Option<u8> {
    INPUT_QUEUE.lock().dequeue_keyboard_byte()
}

pub fn take_terminal_signal() -> Option<u64> {
    let signal = TERMINAL_SIGNAL.swap(0, Ordering::AcqRel);
    (signal != 0).then_some(signal)
}

pub fn requeue_terminal_signal(signal: u64) {
    TERMINAL_SIGNAL.store(signal, Ordering::Release);
}

pub fn key_emits_terminal_sequence(key: KeyCode) -> bool {
    terminal_sequence_for_key(key).is_some()
}

fn terminal_sequence_for_key(key: KeyCode) -> Option<&'static [u8]> {
    match key {
        KeyCode::ArrowUp | KeyCode::Numpad8 => Some(b"\x1b[A"),
        KeyCode::ArrowDown | KeyCode::Numpad2 => Some(b"\x1b[B"),
        KeyCode::ArrowRight | KeyCode::Numpad6 => Some(b"\x1b[C"),
        KeyCode::ArrowLeft | KeyCode::Numpad4 => Some(b"\x1b[D"),
        KeyCode::Home | KeyCode::Numpad7 => Some(b"\x1b[H"),
        KeyCode::End | KeyCode::Numpad1 => Some(b"\x1b[F"),
        KeyCode::Delete | KeyCode::NumpadPeriod => Some(b"\x1b[3~"),
        _ => None,
    }
}

pub fn get_mouse_position() -> MousePosition {
    INPUT_QUEUE.lock().mouse_position()
}

pub fn stats() -> InputStats {
    let queue = INPUT_QUEUE.lock();
    InputStats {
        queue_capacity: EVENT_QUEUE_CAPACITY,
        queued_events: queue.len(),
        keyboard_events: KEYBOARD_EVENTS.load(Ordering::Relaxed),
        typed_char_events: TYPED_CHAR_EVENTS.load(Ordering::Relaxed),
        mouse_move_events: MOUSE_MOVE_EVENTS.load(Ordering::Relaxed),
        mouse_button_events: MOUSE_BUTTON_EVENTS.load(Ordering::Relaxed),
        dropped_events: DROPPED_EVENTS.load(Ordering::Relaxed),
        modifiers: queue.modifiers(),
    }
}

pub fn draw_debug_overlay() {
    let stats = stats();

    crate::vga_buffer::write_status_line(
        0,
        format_args!(
            "IN q={}/{} key={} char={} mmove={} mbtn={} drop={} mods[S{} C{} A{} W{}]",
            stats.queued_events,
            stats.queue_capacity,
            stats.keyboard_events,
            stats.typed_char_events,
            stats.mouse_move_events,
            stats.mouse_button_events,
            stats.dropped_events,
            stats.modifiers.shift as u8,
            stats.modifiers.ctrl as u8,
            stats.modifiers.alt as u8,
            stats.modifiers.super_key as u8,
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::{InputEvent, InputQueue, MouseButton, take_terminal_signal};
    use pc_keyboard::KeyCode;

    #[test_case]
    fn queue_preserves_fifo_order() {
        let mut queue = InputQueue::new();

        queue.enqueue(InputEvent::KeyboardChar('a'));
        queue.enqueue(InputEvent::KeyboardChar('b'));

        assert_eq!(queue.dequeue(), Some(InputEvent::KeyboardChar('a')));
        assert_eq!(queue.dequeue(), Some(InputEvent::KeyboardChar('b')));
        assert_eq!(queue.dequeue(), None);
    }

    #[test_case]
    fn keyboard_modifier_state_tracks_press_and_release() {
        let mut queue = InputQueue::new();

        queue.handle_keyboard_key(KeyCode::LShift, true);
        assert!(queue.modifiers().shift);

        queue.handle_keyboard_key(KeyCode::LShift, false);
        assert!(!queue.modifiers().shift);
    }

    #[test_case]
    fn mouse_button_state_tracks_press_and_release() {
        let mut queue = InputQueue::new();

        queue.handle_mouse_button(MouseButton::Left, true);
        assert!(queue.mouse_left_pressed());

        queue.handle_mouse_button(MouseButton::Left, false);
        assert!(!queue.mouse_left_pressed());
    }

    #[test_case]
    fn special_keys_are_encoded_for_user_stdin() {
        let mut queue = InputQueue::new();

        queue.handle_keyboard_key(KeyCode::ArrowLeft, true);
        queue.handle_keyboard_key(KeyCode::Home, true);
        queue.handle_keyboard_key(KeyCode::Delete, true);

        assert_eq!(queue.dequeue_keyboard_byte(), Some(0x1b));
        assert_eq!(queue.dequeue_keyboard_byte(), Some(b'['));
        assert_eq!(queue.dequeue_keyboard_byte(), Some(b'D'));
        assert_eq!(queue.dequeue_keyboard_byte(), Some(0x1b));
        assert_eq!(queue.dequeue_keyboard_byte(), Some(b'['));
        assert_eq!(queue.dequeue_keyboard_byte(), Some(b'H'));
        assert_eq!(queue.dequeue_keyboard_byte(), Some(0x1b));
        assert_eq!(queue.dequeue_keyboard_byte(), Some(b'['));
        assert_eq!(queue.dequeue_keyboard_byte(), Some(b'3'));
        assert_eq!(queue.dequeue_keyboard_byte(), Some(b'~'));
    }

    #[test_case]
    fn ctrl_c_and_ctrl_z_queue_terminal_signals_without_stdin_bytes() {
        let mut queue = InputQueue::new();
        queue.handle_keyboard_key(KeyCode::LControl, true);
        assert!(queue.handle_keyboard_key(KeyCode::C, true));
        assert_eq!(take_terminal_signal(), Some(crate::user::process::SIGINT));
        assert_eq!(queue.dequeue_keyboard_byte(), None);

        assert!(queue.handle_keyboard_key(KeyCode::Z, true));
        assert_eq!(take_terminal_signal(), Some(crate::user::process::SIGTSTP));
        assert_eq!(queue.dequeue_keyboard_byte(), None);
    }
}
