use std::env;
use std::path::PathBuf;

fn main() {
    let mut arguments = env::args_os().skip(1);
    let kernel = PathBuf::from(arguments.next().expect("kernel ELF path required"));
    let output_directory =
        PathBuf::from(arguments.next().expect("output directory required"));
    assert!(arguments.next().is_none(), "unexpected extra arguments");

    std::fs::create_dir_all(&output_directory).expect("failed to create boot-image output directory");
    let uefi = output_directory.join("vantara-uefi.img");

    bootloader::UefiBoot::new(&kernel)
        .create_disk_image(&uefi)
        .expect("failed to create UEFI image");

    println!("created {}", uefi.display());
}
