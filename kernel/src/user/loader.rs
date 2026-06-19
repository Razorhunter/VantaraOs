use x86_64::VirtAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadedProgram {
    pub path: &'static str,
    pub entry: VirtAddr,
    pub stack_top: VirtAddr,
    pub image_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadError {
    NotFound,
    UnsupportedFormat,
    ImageTooLarge,
    EmptyImage,
}

pub fn load_program(
    program: crate::user::program::UserProgram,
) -> Result<LoadedProgram, LoadError> {
    let image = crate::user::images::find(program.path).ok_or(LoadError::UnsupportedFormat)?;

    if crate::user::elf::parse_elf64(image.data).is_ok() {
        load_elf_program(program, image.data)
    } else {
        load_flat_program(program, image.data)
    }
}

fn load_flat_program(
    program: crate::user::program::UserProgram,
    image: &[u8],
) -> Result<LoadedProgram, LoadError> {
    let path = program.path;

    unsafe {
        crate::user::ring3::load_first_user_task_image(image).map_err(|err| match err {
            crate::user::ring3::UserImageError::EmptyImage => LoadError::EmptyImage,
            crate::user::ring3::UserImageError::ImageTooLarge => LoadError::ImageTooLarge,
        })?;
        crate::user::ring3::seed_first_user_arg(program.arg.as_bytes()).map_err(
            |err| match err {
                crate::user::ring3::UserImageError::EmptyImage => LoadError::EmptyImage,
                crate::user::ring3::UserImageError::ImageTooLarge => LoadError::ImageTooLarge,
            },
        )?;
        let error_message = match program.path {
            "/bin/ls" => crate::user::ring3::FIRST_USER_LS_ERROR,
            _ => crate::user::ring3::FIRST_USER_CAT_ERROR,
        };
        crate::user::ring3::seed_first_user_error_message(error_message).map_err(
            |err| match err {
                crate::user::ring3::UserImageError::EmptyImage => LoadError::EmptyImage,
                crate::user::ring3::UserImageError::ImageTooLarge => LoadError::ImageTooLarge,
            },
        )?;
    }

    Ok(LoadedProgram {
        path,
        entry: VirtAddr::new(crate::user::ring3::FIRST_USER_ENTRY),
        stack_top: VirtAddr::new(crate::user::ring3::FIRST_USER_STACK_TOP),
        image_len: image.len(),
    })
}

fn load_elf_program(
    program: crate::user::program::UserProgram,
    image: &[u8],
) -> Result<LoadedProgram, LoadError> {
    let path = program.path;

    let (entry, image_footprint_size) = unsafe {
        crate::user::ring3::load_user_elf_image(image).map_err(|err| match err {
            crate::user::ring3::UserImageError::EmptyImage => LoadError::EmptyImage,
            crate::user::ring3::UserImageError::ImageTooLarge => LoadError::ImageTooLarge,
        })?
    };

    unsafe {
        crate::user::ring3::seed_first_user_arg(program.arg.as_bytes()).map_err(
            |err| match err {
                crate::user::ring3::UserImageError::EmptyImage => LoadError::EmptyImage,
                crate::user::ring3::UserImageError::ImageTooLarge => LoadError::ImageTooLarge,
            },
        )?;
        crate::user::ring3::seed_first_user_error_message(crate::user::ring3::FIRST_USER_CAT_ERROR)
            .map_err(|err| match err {
                crate::user::ring3::UserImageError::EmptyImage => LoadError::EmptyImage,
                crate::user::ring3::UserImageError::ImageTooLarge => LoadError::ImageTooLarge,
            })?;
    }

    Ok(LoadedProgram {
        path,
        entry: VirtAddr::new(entry),
        stack_top: VirtAddr::new(crate::user::ring3::FIRST_USER_STACK_TOP),
        image_len: image_footprint_size as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::{LoadError, load_program};
    use crate::user::program::{JobMode, UserProgram, UserProgramArg};
    use x86_64::VirtAddr;

    #[test_case]
    fn rejects_unknown_program_format() {
        let program = UserProgram {
            pid: 2,
            parent_pid: Some(1),
            name: "missing",
            path: "/bin/missing",
            entry: VirtAddr::new(crate::user::ring3::FIRST_USER_ENTRY),
            stack_top: VirtAddr::new(crate::user::ring3::FIRST_USER_STACK_TOP),
            arg: UserProgramArg::empty(),
            job_mode: JobMode::Foreground,
        };

        assert_eq!(load_program(program), Err(LoadError::UnsupportedFormat));
    }
}
