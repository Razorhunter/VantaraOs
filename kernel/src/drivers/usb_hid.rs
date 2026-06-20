// USB HID (Human Interface Device) Driver
// Handles USB keyboards and mice

use crate::drivers::usb_structs::*;
use crate::sync::PreemptMutex as Mutex;
use pc_keyboard::KeyCode;

#[repr(u8)]
pub enum HidClassRequest {
    GetReport = 0x01,
    GetIdle = 0x02,
    GetProtocol = 0x03,
    SetReport = 0x09,
    SetIdle = 0x0A,
    SetProtocol = 0x0B,
}

#[repr(u8)]
pub enum HidDescriptorType {
    HID = 0x21,
    Report = 0x22,
    Physical = 0x23,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HidDeviceType {
    Keyboard,
    Mouse,
    Unknown,
}

impl HidDeviceType {
    pub fn from_interface_subclass(subclass: u8, protocol: u8) -> Self {
        // Subclass 0x01 = Boot Interface Subclass
        if subclass == 0x01 {
            match protocol {
                0x01 => HidDeviceType::Keyboard,
                0x02 => HidDeviceType::Mouse,
                _ => HidDeviceType::Unknown,
            }
        } else {
            HidDeviceType::Unknown
        }
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct HidDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub hid_version: u16,
    pub country_code: u8,
    pub num_descriptors: u8,
}

#[derive(Debug, Clone)]
pub struct UsbHidDevice {
    pub device: UsbDevice,
    pub hid_type: HidDeviceType,
    pub interrupt_endpoint: u8,
    pub interval: u8,
    pub max_packet_size: u16,
}

impl UsbHidDevice {
    pub fn new(
        device: UsbDevice,
        interface: &UsbInterfaceDescriptor,
    ) -> Result<Self, &'static str> {
        let hid_type = HidDeviceType::from_interface_subclass(
            interface.interface_subclass,
            interface.interface_protocol,
        );

        if hid_type == HidDeviceType::Unknown {
            return Err("Unknown HID device type");
        }

        Ok(UsbHidDevice {
            device,
            hid_type,
            interrupt_endpoint: 0x81, // Default interrupt IN endpoint
            interval: 10,             // Default interval (10ms)
            max_packet_size: 8,       // Default packet size
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HidKeyboardReport {
    pub modifier: u8,
    pub reserved: u8,
    pub keycodes: [u8; 6],
}

impl HidKeyboardReport {
    pub fn new(data: &[u8]) -> Self {
        let mut keycodes = [0u8; 6];
        if data.len() >= 8 {
            keycodes.copy_from_slice(&data[2..8]);
        }

        HidKeyboardReport {
            modifier: if data.len() > 0 { data[0] } else { 0 },
            reserved: if data.len() > 1 { data[1] } else { 0 },
            keycodes,
        }
    }

    pub fn has_key(&self, keycode: u8) -> bool {
        self.keycodes.contains(&keycode)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HidMouseReport {
    pub button_mask: u8,
    pub x_delta: i8,
    pub y_delta: i8,
}

impl HidMouseReport {
    pub fn new(data: &[u8]) -> Self {
        HidMouseReport {
            button_mask: if data.len() > 0 { data[0] } else { 0 },
            x_delta: if data.len() > 1 { data[1] as i8 } else { 0 },
            y_delta: if data.len() > 2 { data[2] as i8 } else { 0 },
        }
    }

    pub fn left_button(&self) -> bool {
        (self.button_mask & 0x01) != 0
    }

    pub fn right_button(&self) -> bool {
        (self.button_mask & 0x02) != 0
    }

    pub fn middle_button(&self) -> bool {
        (self.button_mask & 0x04) != 0
    }
}

pub fn init_hid_device(device: UsbHidDevice) -> Result<(), &'static str> {
    crate::serial_println!(
        "[HID] Initializing USB HID device: VID=0x{:04X} PID=0x{:04X}",
        device.device.vendor_id,
        device.device.product_id
    );

    match device.hid_type {
        HidDeviceType::Keyboard => {
            crate::serial_println!(
                "[HID] USB Keyboard detected at address {}",
                device.device.address
            );
        }
        HidDeviceType::Mouse => {
            crate::serial_println!(
                "[HID] USB Mouse detected at address {}",
                device.device.address
            );
        }
        HidDeviceType::Unknown => {
            crate::serial_println!("[HID] Unknown HID device type");
            return Err("Unknown HID device type");
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct HidInputState {
    keyboard: HidKeyboardReport,
    mouse_buttons: u8,
    caps_lock: bool,
}

impl HidInputState {
    const fn new() -> Self {
        Self {
            keyboard: HidKeyboardReport {
                modifier: 0,
                reserved: 0,
                keycodes: [0; 6],
            },
            mouse_buttons: 0,
            caps_lock: false,
        }
    }

    fn process_keyboard_report(&mut self, report: &HidKeyboardReport) {
        self.dispatch_modifier_changes(report.modifier);

        for keycode in self.keyboard.keycodes {
            if keycode != 0 && !report.has_key(keycode) {
                if let Some(key) = keycode_to_pc_key(keycode) {
                    crate::input::INPUT_QUEUE
                        .lock()
                        .handle_keyboard_key(key, false);
                }
            }
        }

        for keycode in report.keycodes {
            if keycode == 0 || self.keyboard.has_key(keycode) {
                continue;
            }
            let Some(key) = keycode_to_pc_key(keycode) else {
                continue;
            };

            if key == KeyCode::CapsLock {
                self.caps_lock = !self.caps_lock;
            }

            let mut input = crate::input::INPUT_QUEUE.lock();
            let terminal_control = input.handle_keyboard_key(key, true);
            if !terminal_control && !crate::input::key_emits_terminal_sequence(key) {
                let shift = report.modifier & 0x22 != 0;
                let ctrl = report.modifier & 0x11 != 0;
                if let Some(ch) = keycode_to_char(keycode, shift, ctrl, self.caps_lock) {
                    input.handle_keyboard_char(ch);
                }
            }
        }

        self.keyboard = *report;
    }

    fn dispatch_modifier_changes(&self, modifiers: u8) {
        const MODIFIERS: &[(u8, KeyCode)] = &[
            (0x01, KeyCode::LControl),
            (0x02, KeyCode::LShift),
            (0x04, KeyCode::LAlt),
            (0x08, KeyCode::LWin),
            (0x10, KeyCode::RControl),
            (0x20, KeyCode::RShift),
            (0x40, KeyCode::RAltGr),
            (0x80, KeyCode::RWin),
        ];

        let mut input = crate::input::INPUT_QUEUE.lock();
        for (mask, key) in MODIFIERS {
            let was_pressed = self.keyboard.modifier & mask != 0;
            let is_pressed = modifiers & mask != 0;
            if was_pressed != is_pressed {
                input.handle_keyboard_key(*key, is_pressed);
            }
        }
    }

    fn process_mouse_report(&mut self, report: &HidMouseReport) {
        let mut input = crate::input::INPUT_QUEUE.lock();
        if report.x_delta != 0 || report.y_delta != 0 {
            input.handle_mouse_move(report.x_delta as i16, report.y_delta as i16);
        }

        const BUTTONS: &[(u8, crate::input::MouseButton)] = &[
            (0x01, crate::input::MouseButton::Left),
            (0x02, crate::input::MouseButton::Right),
            (0x04, crate::input::MouseButton::Middle),
        ];
        for (mask, button) in BUTTONS {
            let was_pressed = self.mouse_buttons & mask != 0;
            let is_pressed = report.button_mask & mask != 0;
            if was_pressed != is_pressed {
                input.handle_mouse_button(*button, is_pressed);
            }
        }
        self.mouse_buttons = report.button_mask;
    }
}

static HID_INPUT_STATE: Mutex<HidInputState> = Mutex::new(HidInputState::new());

pub fn process_keyboard_report(report: &HidKeyboardReport) {
    HID_INPUT_STATE.lock().process_keyboard_report(report);
}

pub fn process_mouse_report(report: &HidMouseReport) {
    HID_INPUT_STATE.lock().process_mouse_report(report);
}

fn keycode_to_pc_key(keycode: u8) -> Option<KeyCode> {
    Some(match keycode {
        4 => KeyCode::A,
        5 => KeyCode::B,
        6 => KeyCode::C,
        7 => KeyCode::D,
        8 => KeyCode::E,
        9 => KeyCode::F,
        10 => KeyCode::G,
        11 => KeyCode::H,
        12 => KeyCode::I,
        13 => KeyCode::J,
        14 => KeyCode::K,
        15 => KeyCode::L,
        16 => KeyCode::M,
        17 => KeyCode::N,
        18 => KeyCode::O,
        19 => KeyCode::P,
        20 => KeyCode::Q,
        21 => KeyCode::R,
        22 => KeyCode::S,
        23 => KeyCode::T,
        24 => KeyCode::U,
        25 => KeyCode::V,
        26 => KeyCode::W,
        27 => KeyCode::X,
        28 => KeyCode::Y,
        29 => KeyCode::Z,
        30 => KeyCode::Key1,
        31 => KeyCode::Key2,
        32 => KeyCode::Key3,
        33 => KeyCode::Key4,
        34 => KeyCode::Key5,
        35 => KeyCode::Key6,
        36 => KeyCode::Key7,
        37 => KeyCode::Key8,
        38 => KeyCode::Key9,
        39 => KeyCode::Key0,
        40 => KeyCode::Return,
        41 => KeyCode::Escape,
        42 => KeyCode::Backspace,
        43 => KeyCode::Tab,
        44 => KeyCode::Spacebar,
        45 => KeyCode::OemMinus,
        46 => KeyCode::OemPlus,
        47 => KeyCode::Oem4,
        48 => KeyCode::Oem6,
        49 => KeyCode::Oem5,
        51 => KeyCode::Oem1,
        52 => KeyCode::Oem3,
        53 => KeyCode::Oem8,
        54 => KeyCode::OemComma,
        55 => KeyCode::OemPeriod,
        56 => KeyCode::Oem2,
        57 => KeyCode::CapsLock,
        58 => KeyCode::F1,
        59 => KeyCode::F2,
        60 => KeyCode::F3,
        61 => KeyCode::F4,
        62 => KeyCode::F5,
        63 => KeyCode::F6,
        64 => KeyCode::F7,
        65 => KeyCode::F8,
        66 => KeyCode::F9,
        67 => KeyCode::F10,
        68 => KeyCode::F11,
        69 => KeyCode::F12,
        73 => KeyCode::Insert,
        74 => KeyCode::Home,
        75 => KeyCode::PageUp,
        76 => KeyCode::Delete,
        77 => KeyCode::End,
        78 => KeyCode::PageDown,
        79 => KeyCode::ArrowRight,
        80 => KeyCode::ArrowLeft,
        81 => KeyCode::ArrowDown,
        82 => KeyCode::ArrowUp,
        _ => return None,
    })
}

fn keycode_to_char(keycode: u8, shift: bool, ctrl: bool, caps_lock: bool) -> Option<char> {
    if (4..=29).contains(&keycode) {
        let letter = b'a' + (keycode - 4);
        if ctrl {
            return Some((letter - b'a' + 1) as char);
        }
        let uppercase = shift ^ caps_lock;
        return Some(if uppercase {
            (letter - b'a' + b'A') as char
        } else {
            letter as char
        });
    }

    match (keycode, shift) {
        (30, false) => Some('1'),
        (30, true) => Some('!'),
        (31, false) => Some('2'),
        (31, true) => Some('@'),
        (32, false) => Some('3'),
        (32, true) => Some('#'),
        (33, false) => Some('4'),
        (33, true) => Some('$'),
        (34, false) => Some('5'),
        (34, true) => Some('%'),
        (35, false) => Some('6'),
        (35, true) => Some('^'),
        (36, false) => Some('7'),
        (36, true) => Some('&'),
        (37, false) => Some('8'),
        (37, true) => Some('*'),
        (38, false) => Some('9'),
        (38, true) => Some('('),
        (39, false) => Some('0'),
        (39, true) => Some(')'),
        (40, _) => Some('\n'),
        (42, _) => Some('\u{8}'),
        (43, _) => Some('\t'),
        (44, _) => Some(' '),
        (45, false) => Some('-'),
        (45, true) => Some('_'),
        (46, false) => Some('='),
        (46, true) => Some('+'),
        (47, false) => Some('['),
        (47, true) => Some('{'),
        (48, false) => Some(']'),
        (48, true) => Some('}'),
        (49, false) => Some('\\'),
        (49, true) => Some('|'),
        (51, false) => Some(';'),
        (51, true) => Some(':'),
        (52, false) => Some('\''),
        (52, true) => Some('"'),
        (53, false) => Some('`'),
        (53, true) => Some('~'),
        (54, false) => Some(','),
        (54, true) => Some('<'),
        (55, false) => Some('.'),
        (55, true) => Some('>'),
        (56, false) => Some('/'),
        (56, true) => Some('?'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HidDeviceType, HidKeyboardReport, HidMouseReport, keycode_to_char, keycode_to_pc_key,
    };
    use pc_keyboard::KeyCode;

    #[test_case]
    fn identifies_boot_keyboard_and_mouse_interfaces() {
        assert_eq!(
            HidDeviceType::from_interface_subclass(1, 1),
            HidDeviceType::Keyboard
        );
        assert_eq!(
            HidDeviceType::from_interface_subclass(1, 2),
            HidDeviceType::Mouse
        );
    }

    #[test_case]
    fn maps_usb_keys_to_terminal_characters() {
        assert_eq!(keycode_to_pc_key(4), Some(KeyCode::A));
        assert_eq!(keycode_to_pc_key(80), Some(KeyCode::ArrowLeft));
        assert_eq!(keycode_to_char(4, false, false, false), Some('a'));
        assert_eq!(keycode_to_char(4, true, false, false), Some('A'));
        assert_eq!(keycode_to_char(24, false, true, false), Some('\u{15}'));
        assert_eq!(keycode_to_char(56, true, false, false), Some('?'));
    }

    #[test_case]
    fn parses_boot_reports() {
        let keyboard = HidKeyboardReport::new(&[0x02, 0, 4, 0, 0, 0, 0, 0]);
        assert_eq!(keyboard.modifier, 0x02);
        assert!(keyboard.has_key(4));

        let mouse = HidMouseReport::new(&[0x03, 5, 0xfe]);
        assert!(mouse.left_button());
        assert!(mouse.right_button());
        assert_eq!(mouse.x_delta, 5);
        assert_eq!(mouse.y_delta, -2);
    }
}
