use x86_64::instructions::port::Port;

const DATA_PORT: u16 = 0x60;
const COMMAND_PORT: u16 = 0x64;

const CMD_READ_CONFIG: u8 = 0x20;
const CMD_WRITE_CONFIG: u8 = 0x60;
const CMD_DISABLE_FIRST_PORT: u8 = 0xad;
const CMD_ENABLE_FIRST_PORT: u8 = 0xae;
const CMD_DISABLE_SECOND_PORT: u8 = 0xa7;
const CMD_ENABLE_SECOND_PORT: u8 = 0xa8;

pub fn init_controller() -> Result<(), &'static str> {
    wait_input_clear()?;
    write_command(CMD_DISABLE_FIRST_PORT);
    wait_input_clear()?;
    write_command(CMD_DISABLE_SECOND_PORT);

    flush_output();

    wait_input_clear()?;
    write_command(CMD_READ_CONFIG);
    wait_output_full()?;
    let mut config = read_data();

    // Enable first and second port IRQs. Keep translation enabled because the
    // keyboard driver decodes translated Set 1 scancodes.
    config |= 0x03;
    config |= 0x40;

    wait_input_clear()?;
    write_command(CMD_WRITE_CONFIG);
    wait_input_clear()?;
    write_data(config);

    wait_input_clear()?;
    write_command(CMD_ENABLE_FIRST_PORT);
    wait_input_clear()?;
    write_command(CMD_ENABLE_SECOND_PORT);

    crate::serial_println!("[PS2] Controller initialized");
    Ok(())
}

fn wait_input_clear() -> Result<(), &'static str> {
    let mut command_port = Port::<u8>::new(COMMAND_PORT);

    for _ in 0..100_000 {
        if unsafe { command_port.read() } & 0x02 == 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }

    Err("PS/2 input buffer timeout")
}

fn wait_output_full() -> Result<(), &'static str> {
    let mut command_port = Port::<u8>::new(COMMAND_PORT);

    for _ in 0..100_000 {
        if unsafe { command_port.read() } & 0x01 != 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }

    Err("PS/2 output buffer timeout")
}

fn flush_output() {
    let mut command_port = Port::<u8>::new(COMMAND_PORT);
    let mut data_port = Port::<u8>::new(DATA_PORT);

    while unsafe { command_port.read() } & 0x01 != 0 {
        let _ = unsafe { data_port.read() };
    }
}

fn read_data() -> u8 {
    let mut data_port = Port::<u8>::new(DATA_PORT);
    unsafe { data_port.read() }
}

fn write_data(data: u8) {
    let mut data_port = Port::<u8>::new(DATA_PORT);
    unsafe {
        data_port.write(data);
    }
}

fn write_command(command: u8) {
    let mut command_port = Port::<u8>::new(COMMAND_PORT);
    unsafe {
        command_port.write(command);
    }
}
