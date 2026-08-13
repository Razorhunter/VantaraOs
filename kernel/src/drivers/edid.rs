const EDID_BLOCK_SIZE: usize = 128;
const EDID_HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdidInfo {
    pub manufacturer: [u8; 3],
    pub product_code: u16,
    pub serial_number: u32,
    pub version: u8,
    pub revision: u8,
    pub preferred_width: u16,
    pub preferred_height: u16,
    pub preferred_refresh_millihertz: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdidError {
    TooShort,
    InvalidHeader,
    InvalidChecksum,
    InvalidManufacturer,
    MissingPreferredTiming,
}

pub fn parse(bytes: &[u8]) -> Result<EdidInfo, EdidError> {
    if bytes.len() < EDID_BLOCK_SIZE {
        return Err(EdidError::TooShort);
    }
    let block = &bytes[..EDID_BLOCK_SIZE];
    if block[..EDID_HEADER.len()] != EDID_HEADER {
        return Err(EdidError::InvalidHeader);
    }
    if block.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != 0 {
        return Err(EdidError::InvalidChecksum);
    }

    let manufacturer_code = u16::from_be_bytes([block[8], block[9]]);
    let manufacturer = [
        decode_letter((manufacturer_code >> 10) & 0x1f)?,
        decode_letter((manufacturer_code >> 5) & 0x1f)?,
        decode_letter(manufacturer_code & 0x1f)?,
    ];
    let timing = &block[54..72];
    let pixel_clock_hz = u32::from(u16::from_le_bytes([timing[0], timing[1]])) * 10_000;
    if pixel_clock_hz == 0 {
        return Err(EdidError::MissingPreferredTiming);
    }
    let width = u16::from(timing[2]) | (u16::from(timing[4] & 0xf0) << 4);
    let horizontal_blank = u16::from(timing[3]) | (u16::from(timing[4] & 0x0f) << 8);
    let height = u16::from(timing[5]) | (u16::from(timing[7] & 0xf0) << 4);
    let vertical_blank = u16::from(timing[6]) | (u16::from(timing[7] & 0x0f) << 8);
    let total_pixels = u32::from(width + horizontal_blank)
        .checked_mul(u32::from(height + vertical_blank))
        .ok_or(EdidError::MissingPreferredTiming)?;
    if width == 0 || height == 0 || total_pixels == 0 {
        return Err(EdidError::MissingPreferredTiming);
    }

    Ok(EdidInfo {
        manufacturer,
        product_code: u16::from_le_bytes([block[10], block[11]]),
        serial_number: u32::from_le_bytes([block[12], block[13], block[14], block[15]]),
        version: block[18],
        revision: block[19],
        preferred_width: width,
        preferred_height: height,
        preferred_refresh_millihertz: pixel_clock_hz.saturating_mul(1000) / total_pixels,
    })
}

fn decode_letter(value: u16) -> Result<u8, EdidError> {
    if !(1..=26).contains(&value) {
        return Err(EdidError::InvalidManufacturer);
    }
    Ok(b'A' + value as u8 - 1)
}

#[cfg(test)]
mod tests {
    use super::{EdidError, parse};

    fn valid_edid() -> [u8; 128] {
        let mut bytes = [0u8; 128];
        bytes[..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        // "QEM" encoded as three five-bit EDID manufacturer letters.
        let manufacturer = (17u16 << 10) | (5u16 << 5) | 13u16;
        bytes[8..10].copy_from_slice(&manufacturer.to_be_bytes());
        bytes[10..12].copy_from_slice(&0x1234u16.to_le_bytes());
        bytes[12..16].copy_from_slice(&0x89abcdefu32.to_le_bytes());
        bytes[18] = 1;
        bytes[19] = 4;
        // 1280x800 @ approximately 60 Hz, 83.5 MHz, totals 1680x831.
        bytes[54..56].copy_from_slice(&8350u16.to_le_bytes());
        bytes[56] = 0x00;
        bytes[57] = 0x90;
        bytes[58] = 0x51;
        bytes[59] = 0x20;
        bytes[60] = 0x1f;
        bytes[61] = 0x30;
        let sum = bytes[..127]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        bytes[127] = 0u8.wrapping_sub(sum);
        bytes
    }

    #[test_case]
    fn parses_preferred_timing_and_identity() {
        let info = parse(&valid_edid()).unwrap();
        assert_eq!(&info.manufacturer, b"QEM");
        assert_eq!(info.product_code, 0x1234);
        assert_eq!(info.preferred_width, 1280);
        assert_eq!(info.preferred_height, 800);
        assert!((59_000..=61_000).contains(&info.preferred_refresh_millihertz));
    }

    #[test_case]
    fn rejects_bad_checksum() {
        let mut bytes = valid_edid();
        bytes[20] ^= 1;
        assert_eq!(parse(&bytes), Err(EdidError::InvalidChecksum));
    }
}
