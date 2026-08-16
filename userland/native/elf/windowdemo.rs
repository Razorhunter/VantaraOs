#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    abi::write("windowdemo: protocol start\n");

    if abi::surface_set_color(1, 4) != abi::ERR_PERMISSION_DENIED {
        fail("windowdemo: ownership isolation failed\n");
    }
    abi::write("windowdemo: ownership protected\n");

    let id = abi::surface_create(360, 220, 320, 200, 9);
    if id < 0 {
        fail("windowdemo: create failed\n");
    }
    abi::write("windowdemo: create ok\n");

    let id = id as u64;
    if abi::surface_configure(id, 420, 260, 360, 220) < 0 {
        fail("windowdemo: configure failed\n");
    }
    if abi::surface_set_color(id, 10) < 0 {
        fail("windowdemo: color failed\n");
    }
    if abi::surface_focus(id) < 0 {
        fail("windowdemo: focus failed\n");
    }
    abi::write("windowdemo: configure color focus ok\n");

    let fence = abi::surface_damage(id, 16, 16, 64, 48);
    if fence <= 0 || abi::frame_fence_status(fence as u64) != 1 {
        fail("windowdemo: damage fence failed\n");
    }
    abi::write("windowdemo: damage fence complete\n");

    if abi::surface_destroy(id) < 0 {
        fail("windowdemo: destroy failed\n");
    }
    abi::write("windowdemo: destroy ok\n");

    if abi::surface_create(80, 80, 160, 100, 13) < 0 {
        fail("windowdemo: exit cleanup setup failed\n");
    }
    abi::write("windowdemo: exit cleanup armed\n");
    abi::write("windowdemo: PASS\n");
    abi::exit(0);
}

fn fail(message: &str) -> ! {
    abi::write(message);
    abi::exit(1);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(1);
}
