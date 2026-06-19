const ELF_MAGIC: &[u8; 4] = b"\x7fELF";
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 0x3e;
const ELF_VERSION_CURRENT: u32 = 1;
const ELF64_PROGRAM_HEADER_SIZE: u16 = 56;
const PT_LOAD: u32 = 1;
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;
const USER_ELF_MIN_VADDR: u64 = crate::user::ring3::FIRST_USER_ENTRY;
const USER_ELF_MAX_VADDR: u64 =
    crate::user::ring3::FIRST_USER_STACK_TOP - crate::user::ring3::USER_STACK_SIZE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElfInfo {
    pub entry: u64,
    pub program_header_offset: u64,
    pub program_header_entry_size: u16,
    pub program_header_count: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadSegment {
    pub file_offset: u64,
    pub virtual_address: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub flags: u32,
    pub alignment: u64,
}

impl LoadSegment {
    pub const EMPTY: Self = Self {
        file_offset: 0,
        virtual_address: 0,
        file_size: 0,
        memory_size: 0,
        flags: 0,
        alignment: 0,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfError {
    TooSmall,
    BadMagic,
    UnsupportedClass,
    UnsupportedEndian,
    UnsupportedVersion,
    UnsupportedType,
    UnsupportedMachine,
    BadProgramHeaderSize,
    ProgramHeadersOutOfBounds,
    SegmentOutOfBounds,
    SegmentAddressOutOfBounds,
    SegmentMemoryTooSmall,
    BadSegmentFlags,
    BadSegmentAlignment,
    TooManyLoadSegments,
}

pub fn segment_page_flags(
    segment: LoadSegment,
) -> Result<x86_64::structures::paging::PageTableFlags, ElfError> {
    validate_user_load_segment(segment)?;
    let mut flags = x86_64::structures::paging::PageTableFlags::PRESENT
        | x86_64::structures::paging::PageTableFlags::USER_ACCESSIBLE;
    if segment.flags & PF_W != 0 {
        flags |= x86_64::structures::paging::PageTableFlags::WRITABLE;
    }
    if segment.flags & PF_X == 0 {
        flags |= x86_64::structures::paging::PageTableFlags::NO_EXECUTE;
    }
    Ok(flags)
}

pub fn parse_elf64(image: &[u8]) -> Result<ElfInfo, ElfError> {
    if image.len() < 64 {
        return Err(ElfError::TooSmall);
    }
    if &image[0..4] != ELF_MAGIC {
        return Err(ElfError::BadMagic);
    }
    if image[4] != ELFCLASS64 {
        return Err(ElfError::UnsupportedClass);
    }
    if image[5] != ELFDATA2LSB {
        return Err(ElfError::UnsupportedEndian);
    }
    if read_u32(image, 20) != ELF_VERSION_CURRENT {
        return Err(ElfError::UnsupportedVersion);
    }

    let elf_type = read_u16(image, 16);
    if elf_type != ET_EXEC {
        return Err(ElfError::UnsupportedType);
    }

    let machine = read_u16(image, 18);
    if machine != EM_X86_64 {
        return Err(ElfError::UnsupportedMachine);
    }

    let program_header_offset = read_u64(image, 32);
    let program_header_entry_size = read_u16(image, 54);
    let program_header_count = read_u16(image, 56);
    validate_program_header_table(
        image,
        program_header_offset,
        program_header_entry_size,
        program_header_count,
    )?;

    Ok(ElfInfo {
        entry: read_u64(image, 24),
        program_header_offset,
        program_header_entry_size,
        program_header_count,
    })
}

pub fn load_segments(image: &[u8], output: &mut [LoadSegment]) -> Result<usize, ElfError> {
    let info = parse_elf64(image)?;
    let mut count = 0usize;

    for index in 0..info.program_header_count {
        let offset = info.program_header_offset as usize
            + index as usize * info.program_header_entry_size as usize;
        let program_type = read_u32(image, offset);
        if program_type != PT_LOAD {
            continue;
        }

        if count >= output.len() {
            return Err(ElfError::TooManyLoadSegments);
        }

        let segment = LoadSegment {
            flags: read_u32(image, offset + 4),
            file_offset: read_u64(image, offset + 8),
            virtual_address: read_u64(image, offset + 16),
            file_size: read_u64(image, offset + 32),
            memory_size: read_u64(image, offset + 40),
            alignment: read_u64(image, offset + 48),
        };

        validate_load_segment(image, segment)?;
        validate_user_load_segment(segment)?;
        output[count] = segment;
        count += 1;
    }

    Ok(count)
}

pub fn image_memory_size(segments: &[LoadSegment]) -> u64 {
    let mut low = u64::MAX;
    let mut high = 0u64;

    for segment in segments {
        if segment.memory_size == 0 {
            continue;
        }
        low = low.min(segment.virtual_address);
        high = high.max(segment.virtual_address.saturating_add(segment.memory_size));
    }

    if low == u64::MAX {
        0
    } else {
        high.saturating_sub(low)
    }
}

pub fn image_footprint_size(segments: &[LoadSegment]) -> u64 {
    let mut high = USER_ELF_MIN_VADDR;

    for segment in segments {
        if segment.memory_size == 0 {
            continue;
        }
        high = high.max(segment.virtual_address.saturating_add(segment.memory_size));
    }

    high.saturating_sub(USER_ELF_MIN_VADDR)
}

pub unsafe fn materialize_load_segments(
    image: &[u8],
    segments: &[LoadSegment],
) -> Result<(), ElfError> {
    for segment in segments {
        validate_load_segment(image, *segment)?;
        validate_user_load_segment(*segment)?;

        let file_start = segment.file_offset as usize;
        let file_end = file_start + segment.file_size as usize;
        let destination = segment.virtual_address as *mut u8;

        unsafe {
            core::ptr::copy_nonoverlapping(
                image[file_start..file_end].as_ptr(),
                destination,
                segment.file_size as usize,
            );
            if segment.memory_size > segment.file_size {
                core::ptr::write_bytes(
                    destination.add(segment.file_size as usize),
                    0,
                    (segment.memory_size - segment.file_size) as usize,
                );
            }
        }
    }

    Ok(())
}

fn validate_program_header_table(
    image: &[u8],
    offset: u64,
    entry_size: u16,
    count: u16,
) -> Result<(), ElfError> {
    if count == 0 {
        return Ok(());
    }
    if entry_size < ELF64_PROGRAM_HEADER_SIZE {
        return Err(ElfError::BadProgramHeaderSize);
    }

    let offset = offset as usize;
    let size = entry_size as usize * count as usize;
    let end = offset
        .checked_add(size)
        .ok_or(ElfError::ProgramHeadersOutOfBounds)?;
    if end > image.len() {
        return Err(ElfError::ProgramHeadersOutOfBounds);
    }

    Ok(())
}

fn validate_load_segment(image: &[u8], segment: LoadSegment) -> Result<(), ElfError> {
    if segment.memory_size < segment.file_size {
        return Err(ElfError::SegmentMemoryTooSmall);
    }

    let offset = segment.file_offset as usize;
    let size = segment.file_size as usize;
    let end = offset
        .checked_add(size)
        .ok_or(ElfError::SegmentOutOfBounds)?;
    if end > image.len() {
        return Err(ElfError::SegmentOutOfBounds);
    }

    Ok(())
}

fn validate_user_load_segment(segment: LoadSegment) -> Result<(), ElfError> {
    if segment.flags & !(PF_R | PF_W | PF_X) != 0 {
        return Err(ElfError::BadSegmentFlags);
    }
    if segment.flags & PF_W != 0 && segment.flags & PF_X != 0 {
        return Err(ElfError::BadSegmentFlags);
    }

    if segment.alignment > 1 && !segment.alignment.is_power_of_two() {
        return Err(ElfError::BadSegmentAlignment);
    }

    if segment.virtual_address < USER_ELF_MIN_VADDR {
        return Err(ElfError::SegmentAddressOutOfBounds);
    }

    let end = segment
        .virtual_address
        .checked_add(segment.memory_size)
        .ok_or(ElfError::SegmentAddressOutOfBounds)?;
    if end > USER_ELF_MAX_VADDR {
        return Err(ElfError::SegmentAddressOutOfBounds);
    }

    Ok(())
}

fn read_u16(image: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([image[offset], image[offset + 1]])
}

fn read_u32(image: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        image[offset],
        image[offset + 1],
        image[offset + 2],
        image[offset + 3],
    ])
}

fn read_u64(image: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        image[offset],
        image[offset + 1],
        image[offset + 2],
        image[offset + 3],
        image[offset + 4],
        image[offset + 5],
        image[offset + 6],
        image[offset + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::{
        ElfError, LoadSegment, PF_R, PF_W, PF_X, load_segments, parse_elf64, segment_page_flags,
    };
    use x86_64::structures::paging::PageTableFlags;

    #[test_case]
    fn rejects_bad_magic() {
        let image = [0u8; 64];

        assert_eq!(parse_elf64(&image), Err(ElfError::BadMagic));
    }

    #[test_case]
    fn parses_executable_header() {
        let image = sample_elf_with_load_segment();

        let info = parse_elf64(&image).expect("ELF header should parse");

        assert_eq!(info.entry, crate::user::ring3::FIRST_USER_ENTRY);
        assert_eq!(info.program_header_offset, 64);
        assert_eq!(info.program_header_entry_size, 56);
        assert_eq!(info.program_header_count, 1);
    }

    #[test_case]
    fn derives_wx_safe_page_permissions() {
        let executable = LoadSegment {
            flags: PF_R | PF_X,
            memory_size: 4096,
            ..LoadSegment::EMPTY
        };
        let writable = LoadSegment {
            flags: PF_R | PF_W,
            memory_size: 4096,
            ..LoadSegment::EMPTY
        };

        let code_flags = segment_page_flags(executable).unwrap();
        assert!(!code_flags.contains(PageTableFlags::WRITABLE));
        assert!(!code_flags.contains(PageTableFlags::NO_EXECUTE));

        let data_flags = segment_page_flags(writable).unwrap();
        assert!(data_flags.contains(PageTableFlags::WRITABLE));
        assert!(data_flags.contains(PageTableFlags::NO_EXECUTE));
    }

    #[test_case]
    fn rejects_writable_executable_segment() {
        let segment = LoadSegment {
            flags: PF_R | PF_W | PF_X,
            memory_size: 4096,
            ..LoadSegment::EMPTY
        };

        assert_eq!(segment_page_flags(segment), Err(ElfError::BadSegmentFlags));
    }

    #[test_case]
    fn parses_load_segments() {
        let image = sample_elf_with_load_segment();
        let mut segments = [LoadSegment::EMPTY; 2];

        let count = load_segments(&image, &mut segments).expect("load segment should parse");

        assert_eq!(count, 1);
        assert_eq!(segments[0].file_offset, 0x100);
        assert_eq!(
            segments[0].virtual_address,
            crate::user::ring3::FIRST_USER_ENTRY
        );
        assert_eq!(segments[0].file_size, 4);
        assert_eq!(segments[0].memory_size, 8);
        assert_eq!(segments[0].flags, 5);
        assert_eq!(segments[0].alignment, 0x1000);
    }

    #[test_case]
    fn rejects_segment_with_bss_smaller_than_file_data() {
        let mut image = sample_elf_with_load_segment();
        write_u64(&mut image, 64 + 40, 2);
        let mut segments = [LoadSegment::EMPTY; 1];

        assert_eq!(
            load_segments(&image, &mut segments),
            Err(ElfError::SegmentMemoryTooSmall)
        );
    }

    #[test_case]
    fn rejects_load_segment_outside_user_range() {
        let mut image = sample_elf_with_load_segment();
        write_u64(&mut image, 64 + 16, 0x1000);
        let mut segments = [LoadSegment::EMPTY; 1];

        assert_eq!(
            load_segments(&image, &mut segments),
            Err(ElfError::SegmentAddressOutOfBounds)
        );
    }

    #[test_case]
    fn rejects_bad_segment_flags() {
        let mut image = sample_elf_with_load_segment();
        write_u32(&mut image, 64 + 4, 0x80);
        let mut segments = [LoadSegment::EMPTY; 1];

        assert_eq!(
            load_segments(&image, &mut segments),
            Err(ElfError::BadSegmentFlags)
        );
    }

    #[test_case]
    fn calculates_loaded_image_memory_size() {
        let image = sample_elf_with_load_segment();
        let mut segments = [LoadSegment::EMPTY; 2];
        let count = load_segments(&image, &mut segments).expect("load segment should parse");

        assert_eq!(super::image_memory_size(&segments[..count]), 8);
        assert_eq!(super::image_footprint_size(&segments[..count]), 8);
    }

    #[test_case]
    fn footprint_includes_gap_from_user_base() {
        let segment = LoadSegment {
            file_offset: 0,
            virtual_address: crate::user::ring3::FIRST_USER_ENTRY + 0x1000,
            file_size: 4,
            memory_size: 8,
            flags: 5,
            alignment: 0x1000,
        };

        assert_eq!(super::image_memory_size(&[segment]), 8);
        assert_eq!(super::image_footprint_size(&[segment]), 0x1008);
    }

    fn sample_elf_with_load_segment() -> [u8; 0x104] {
        let mut image = [0u8; 0x104];
        image[0..4].copy_from_slice(b"\x7fELF");
        image[4] = 2;
        image[5] = 1;
        image[6] = 1;
        write_u16(&mut image, 16, 2);
        write_u16(&mut image, 18, 0x3e);
        write_u32(&mut image, 20, 1);
        write_u64(&mut image, 24, crate::user::ring3::FIRST_USER_ENTRY);
        write_u64(&mut image, 32, 64);
        write_u16(&mut image, 54, 56);
        write_u16(&mut image, 56, 1);

        write_u32(&mut image, 64, 1);
        write_u32(&mut image, 64 + 4, 5);
        write_u64(&mut image, 64 + 8, 0x100);
        write_u64(&mut image, 64 + 16, crate::user::ring3::FIRST_USER_ENTRY);
        write_u64(&mut image, 64 + 32, 4);
        write_u64(&mut image, 64 + 40, 8);
        write_u64(&mut image, 64 + 48, 0x1000);
        image[0x100..0x104].copy_from_slice(&[0x90, 0x90, 0xeb, 0xfe]);

        image
    }

    fn write_u16(image: &mut [u8], offset: usize, value: u16) {
        image[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(image: &mut [u8], offset: usize, value: u32) {
        image[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u64(image: &mut [u8], offset: usize, value: u64) {
        image[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
}
