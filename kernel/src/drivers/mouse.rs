use crate::sync::PreemptMutex as Mutex;
use x86_64::instructions::port::Port;

use crate::input::{INPUT_QUEUE, MouseButton};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MousePacket {
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
    pub x_overflow: bool,
    pub y_overflow: bool,
    pub x_movement: i16,
    pub y_movement: i16,
}

pub trait MouseDriver {
    fn on_interrupt(&self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ButtonState {
    left: bool,
    right: bool,
    middle: bool,
}

impl ButtonState {
    const fn new() -> Self {
        Self {
            left: false,
            right: false,
            middle: false,
        }
    }

    const fn from_packet(packet: MousePacket) -> Self {
        Self {
            left: packet.left_button,
            right: packet.right_button,
            middle: packet.middle_button,
        }
    }
}

pub struct Ps2Mouse {
    packet_bytes: Mutex<[u8; 3]>,
    byte_index: Mutex<usize>,
    last_buttons: Mutex<ButtonState>,
}

impl Ps2Mouse {
    pub const fn new() -> Self {
        Ps2Mouse {
            packet_bytes: Mutex::new([0; 3]),
            byte_index: Mutex::new(0),
            last_buttons: Mutex::new(ButtonState::new()),
        }
    }

    fn read_data() -> u8 {
        let mut port = Port::new(0x60);
        unsafe { port.read() }
    }

    pub fn init(&self) -> Result<(), &'static str> {
        Self::wait_input_clear()?;
        Self::write_command(0xa8);

        Self::wait_input_clear()?;
        Self::write_command(0x20);
        Self::wait_output_full()?;
        let status = Self::read_data() | 0x02;

        Self::wait_input_clear()?;
        Self::write_command(0x60);
        Self::wait_input_clear()?;
        Self::write_data(status);

        Self::write_mouse_command(0xf6)?;
        Self::write_mouse_command(0xf4)?;
        Ok(())
    }

    fn write_mouse_command(command: u8) -> Result<(), &'static str> {
        Self::wait_input_clear()?;
        Self::write_command(0xd4);
        Self::wait_input_clear()?;
        Self::write_data(command);
        Self::wait_output_full()?;

        match Self::read_data() {
            0xfa => Ok(()),
            _ => Err("PS/2 mouse command not acknowledged"),
        }
    }

    fn wait_input_clear() -> Result<(), &'static str> {
        let mut command_port = Port::<u8>::new(0x64);

        for _ in 0..100_000 {
            if unsafe { command_port.read() } & 0x02 == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }

        Err("PS/2 controller input buffer timeout")
    }

    fn wait_output_full() -> Result<(), &'static str> {
        let mut command_port = Port::<u8>::new(0x64);

        for _ in 0..100_000 {
            if unsafe { command_port.read() } & 0x01 != 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }

        Err("PS/2 controller output buffer timeout")
    }

    fn write_command(command: u8) {
        let mut command_port = Port::<u8>::new(0x64);
        unsafe {
            command_port.write(command);
        }
    }

    fn write_data(data: u8) {
        let mut data_port = Port::<u8>::new(0x60);
        unsafe {
            data_port.write(data);
        }
    }

    fn parse_packet(bytes: [u8; 3]) -> MousePacket {
        let byte0 = bytes[0];
        let byte1 = bytes[1];
        let byte2 = bytes[2];

        // Byte 0: [Y_OVERFLOW | X_OVERFLOW | Y_SIGN | X_SIGN | 1 | MIDDLE | RIGHT | LEFT]
        let left_button = (byte0 & 0x01) != 0;
        let right_button = (byte0 & 0x02) != 0;
        let middle_button = (byte0 & 0x04) != 0;
        let x_overflow = (byte0 & 0x40) != 0;
        let y_overflow = (byte0 & 0x80) != 0;

        // Byte 1: X displacement
        let x_movement = if (byte0 & 0x10) != 0 {
            // X sign bit set - negative movement (sign extend)
            (byte1 as i16) | (0xFF00u16 as i16)
        } else {
            byte1 as i16
        };

        // Byte 2: Y displacement
        let y_movement = if (byte0 & 0x20) != 0 {
            // Y sign bit set - negative movement (sign extend)
            (byte2 as i16) | (0xFF00u16 as i16)
        } else {
            byte2 as i16
        };

        MousePacket {
            left_button,
            right_button,
            middle_button,
            x_overflow,
            y_overflow,
            x_movement,
            y_movement,
        }
    }

    fn dispatch_packet(&self, packet: MousePacket) {
        let mut input_queue = INPUT_QUEUE.lock();

        if (packet.x_movement != 0 || packet.y_movement != 0)
            && !packet.x_overflow
            && !packet.y_overflow
        {
            input_queue.handle_mouse_move(packet.x_movement, packet.y_movement);
        }

        let current_buttons = ButtonState::from_packet(packet);
        let mut last_buttons = self.last_buttons.lock();

        if current_buttons.left != last_buttons.left {
            input_queue.handle_mouse_button(MouseButton::Left, current_buttons.left);
        }
        if current_buttons.right != last_buttons.right {
            input_queue.handle_mouse_button(MouseButton::Right, current_buttons.right);
        }
        if current_buttons.middle != last_buttons.middle {
            input_queue.handle_mouse_button(MouseButton::Middle, current_buttons.middle);
        }

        *last_buttons = current_buttons;
    }
}

impl MouseDriver for Ps2Mouse {
    fn on_interrupt(&self) {
        let data = Self::read_data();

        let packet = {
            let mut index = self.byte_index.lock();

            if *index == 0 && (data & 0x08) == 0 {
                *index = 0;
                return;
            }

            let mut bytes = self.packet_bytes.lock();
            bytes[*index] = data;
            *index += 1;

            if *index < bytes.len() {
                return;
            }

            *index = 0;
            Self::parse_packet(*bytes)
        };

        self.dispatch_packet(packet);

        let _ = packet;
    }
}

pub static MOUSE: Ps2Mouse = Ps2Mouse::new();

#[cfg(test)]
mod tests {
    use super::Ps2Mouse;

    #[test_case]
    fn parses_positive_movement_packet() {
        let packet = Ps2Mouse::parse_packet([0x08, 5, 7]);

        assert_eq!(packet.x_movement, 5);
        assert_eq!(packet.y_movement, 7);
        assert!(!packet.left_button);
    }

    #[test_case]
    fn parses_negative_movement_packet() {
        let packet = Ps2Mouse::parse_packet([0x38, 0xfe, 0xff]);

        assert_eq!(packet.x_movement, -2);
        assert_eq!(packet.y_movement, -1);
    }

    #[test_case]
    fn parses_button_bits() {
        let packet = Ps2Mouse::parse_packet([0x0f, 0, 0]);

        assert!(packet.left_button);
        assert!(packet.right_button);
        assert!(packet.middle_button);
    }
}
