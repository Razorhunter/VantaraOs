use crate::serial_println;

const DEBUG_INPUT_EVENTS: bool = false;
const DEBUG_INPUT_OVERLAY: bool = false;

pub extern "C" fn kernel_event_thread() -> ! {
    serial_println!("[KTHREAD] runtime event loop started");
    run_event_loop();
}

pub fn run_event_loop() -> ! {
    loop {
        crate::drivers::network::poll_runtime();

        if let Some(event) = crate::input::dequeue_event() {
            if DEBUG_INPUT_EVENTS {
                serial_println!("Input event: {:?}", event);
            }
            if !crate::user::process::userland_owns_keyboard() {
                crate::shell::handle_event(&event);
            }

            let pos = crate::input::get_mouse_position();
            crate::vga_buffer::update_mouse_cursor(pos.x as usize, pos.y as usize);
            if DEBUG_INPUT_OVERLAY {
                crate::input::draw_debug_overlay();
            }
        }

        run_pending_user_program();
        run_ready_user_process();
        crate::hlt_loop_once();
    }
}

fn run_pending_user_program() {
    let Some(program) = crate::user::program::take_pending() else {
        return;
    };

    let writable_size = user_image_writable_size(program.path);
    if let Err(err) = unsafe {
        crate::user::address_space::ensure_active_user_program_mapping_writable(writable_size)
    } {
        crate::println!(
            "run: {}: user mapping prepare failed: {:?}",
            program.path,
            err
        );
        crate::serial_println!(
            "[USER] failed to prepare shared user mapping for {}: {:?}",
            program.path,
            err
        );
        crate::shell::prompt();
        return;
    }

    match crate::user::loader::load_program(program) {
        Ok(loaded) => {
            let context = crate::user::process::register_exec(program, loaded);
            if !context.p4_verified {
                if let Err(err) = unsafe {
                    crate::user::address_space::ensure_active_user_stack_mapping_writable(
                        context.layout.stack_start,
                        context.layout.stack_top,
                    )
                } {
                    crate::serial_println!(
                        "[USER] shared user stack prepare skipped for pid={}: {:?}",
                        context.pid,
                        err
                    );
                } else if let Err(err) =
                    unsafe { crate::user::ring3::clear_user_stack_slot(context.layout.stack_top) }
                {
                    crate::serial_println!(
                        "[USER] shared user stack clear skipped for pid={}: {:?}",
                        context.pid,
                        err
                    );
                } else if let Err(err) = unsafe {
                    crate::user::ring3::seed_user_initial_stack(
                        context.layout,
                        context.program_path,
                        context.arg.as_bytes(),
                    )
                } {
                    crate::serial_println!(
                        "[USER] shared initial stack seed skipped for pid={}: {:?}",
                        context.pid,
                        err
                    );
                }
            }
            crate::serial_println!(
                "[USER] loaded pid={} {} bytes={} entry={:#x} stack_top={:#x}",
                context.pid,
                loaded.path,
                loaded.image_len,
                context.entry_point,
                context.user_stack_top
            );
            if context.p4_verified {
                if let Some(frame) = context.p4_frame {
                    let image = crate::user::images::find(loaded.path)
                        .map(|image| image.data)
                        .unwrap_or(&[]);
                    crate::serial_println!(
                        "[USER] preparing private memory pid={} P4 {:#x} path={}",
                        context.pid,
                        frame,
                        loaded.path
                    );
                    if let Err(err) = unsafe {
                        crate::user::address_space::prepare_process_private_memory(
                            frame,
                            context.layout,
                            image,
                            context.program_path,
                            context.arg.as_bytes(),
                        )
                    } {
                        crate::println!(
                            "run: {}: private memory prepare failed: {:?}",
                            loaded.path,
                            err
                        );
                        crate::serial_println!(
                            "[USER] failed to prepare private memory pid={} P4 {:#x}: {:?}",
                            context.pid,
                            frame,
                            err
                        );
                        crate::shell::prompt();
                        return;
                    }
                    crate::serial_println!(
                        "[USER] prepared private user pages for pid={} P4 {:#x}",
                        context.pid,
                        frame
                    );
                    crate::user::process::mark_process_isolated(context.pid);
                    crate::serial_println!(
                        "[USER] switching to pid={} P4 {:#x}",
                        context.pid,
                        frame
                    );
                    unsafe {
                        crate::user::ring3::switch_to_user_p4_and_jump(
                            frame,
                            x86_64::VirtAddr::new(context.entry_point),
                            x86_64::VirtAddr::new(context.user_stack_top),
                        );
                    }
                }
            }
            unsafe {
                crate::user::ring3::jump_to_user(
                    x86_64::VirtAddr::new(context.entry_point),
                    x86_64::VirtAddr::new(context.user_stack_top),
                );
            }
        }
        Err(err) => {
            crate::println!("run: {}: load failed: {:?}", program.path, err);
            crate::serial_println!("[USER] failed to load {}: {:?}", program.path, err);
            crate::shell::prompt();
        }
    }
}

fn run_ready_user_process() {
    if let Some(signal) = crate::input::take_terminal_signal() {
        if let Some(report) = crate::user::process::deliver_terminal_signal(signal) {
            crate::serial_println!(
                "[TTY] signal={} pid={} name={} status={} stopped={} parent_woken={}",
                report.signal,
                report.pid,
                report.name,
                report.status,
                report.stopped,
                report.parent_woken
            );
        } else if crate::user::process::terminal_foreground_job().is_some() {
            crate::input::requeue_terminal_signal(signal);
        } else {
            crate::serial_println!("[TTY] ignored signal={} without foreground job", signal);
        }
    }

    let woken = crate::user::process::wake_sleeping_processes(crate::timer::ticks());
    if woken > 0 {
        crate::serial_println!("[USER] woke {} sleeping user process(es)", woken);
    }

    let Some(resume) = crate::user::process::schedule_next_ready_user() else {
        return;
    };

    if let Some(frame) = resume.p4_frame {
        crate::serial_println!(
            "[USER] runtime scheduler resuming pid={} {} rip={:#x} rsp={:#x} P4 {:#x}",
            resume.pid,
            resume.name,
            resume.resume_context.rip,
            resume.resume_context.rsp,
            frame
        );
        unsafe {
            crate::user::ring3::switch_to_user_p4_and_resume(
                frame,
                resume.resume_context,
                resume.resume_context.rax,
            );
        }
    }

    unsafe {
        crate::user::ring3::resume_user(resume.resume_context, resume.resume_context.rax);
    }
}

fn user_image_writable_size(path: &str) -> u64 {
    let Some(image) = crate::user::images::find(path) else {
        return crate::user::ring3::FIRST_USER_PAGE_SIZE as u64;
    };

    if crate::user::elf::parse_elf64(image.data).is_ok() {
        let mut segments = [crate::user::elf::LoadSegment::EMPTY; 8];
        if let Ok(count) = crate::user::elf::load_segments(image.data, &mut segments) {
            return crate::user::elf::image_footprint_size(&segments[..count]);
        }
    }

    image.data.len() as u64
}
