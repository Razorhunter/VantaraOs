use crate::sync::PreemptMutex as Mutex;
use lazy_static::lazy_static;
use pc_keyboard::{DecodedKey, HandleControl, KeyEvent, KeyState, Keyboard, ScancodeSet1, layouts};
use x86_64::instructions::port::Port;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyType {
    Unicode(char),
    RawKey(pc_keyboard::KeyCode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardEvent {
    pub key: KeyType,
    pub key_event: KeyEvent,
}

/// Trait: apa-apa implementasi keyboard mesti boleh "handle interrupt"
pub trait KeyboardDriver {
    fn on_interrupt(&self);
}

/// Implementasi standard: baca scancode port 0x60 dan decode guna pc_keyboard
pub struct Ps2Keyboard;

lazy_static! {
    static ref KEYBOARD: Mutex<Keyboard<layouts::Us104Key, ScancodeSet1>> =
        Mutex::new(Keyboard::new(
            ScancodeSet1::new(),
            layouts::Us104Key,
            HandleControl::Ignore,
        ));
}

impl Ps2Keyboard {
    pub fn new() -> Self {
        Ps2Keyboard
    }

    pub fn read_scancode() -> u8 {
        let mut port = Port::new(0x60);
        unsafe { port.read() }
    }
}

impl KeyboardDriver for Ps2Keyboard {
    fn on_interrupt(&self) {
        let scancode = Self::read_scancode();

        let mut keyboard = KEYBOARD.lock();
        if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
            let key_code = key_event.code;
            let pressed = matches!(key_event.state, KeyState::Down | KeyState::SingleShot);
            let terminal_control = crate::input::INPUT_QUEUE
                .lock()
                .handle_keyboard_key(key_code, pressed);

            if let Some(key) = keyboard.process_keyevent(key_event) {
                if let DecodedKey::Unicode(ch) = key {
                    if !terminal_control && !crate::input::key_emits_terminal_sequence(key_code) {
                        crate::input::INPUT_QUEUE.lock().handle_keyboard_char(ch);
                    }
                }

                let _ = key;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pc_keyboard::{
        DecodedKey, HandleControl, KeyCode, KeyState, Keyboard, ScancodeSet1, layouts,
    };

    #[test_case]
    fn set1_scancode_decodes_ascii_key() {
        let mut keyboard = Keyboard::new(
            ScancodeSet1::new(),
            layouts::Us104Key,
            HandleControl::Ignore,
        );

        let key_event = keyboard.add_byte(0x23).unwrap().unwrap();
        assert_eq!(key_event.code, KeyCode::H);
        assert_eq!(key_event.state, KeyState::Down);
        assert_eq!(
            keyboard.process_keyevent(key_event),
            Some(DecodedKey::Unicode('h'))
        );
    }

    #[test_case]
    fn set1_release_scancode_decodes_key_up() {
        let mut keyboard = Keyboard::new(
            ScancodeSet1::new(),
            layouts::Us104Key,
            HandleControl::Ignore,
        );

        let key_event = keyboard.add_byte(0xa3).unwrap().unwrap();
        assert_eq!(key_event.code, KeyCode::H);
        assert_eq!(key_event.state, KeyState::Up);
    }
}
