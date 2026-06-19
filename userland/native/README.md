# Vantara Native Userland

This directory contains Vantara-native command sources that are meant to become
`/bin` programs once the kernel has a real user program loader.

The older `userland/ls`, `userland/cat`, and `userland/uptime` directories are
Linux-hosted command implementations. They are useful as feature references, but
they depend on Linux facilities such as `std::fs`, `/proc`, `chrono`, `sysinfo`,
`users`, and `libc`.

The code here is intentionally small and `no_std`:

- `src/abi.rs` is the syscall-facing ABI layer.
- `src/commands/uptime.rs` is the native `uptime` command logic.
- `src/commands/ls.rs` is the native `ls` command logic.
- `src/commands/cat.rs` is the native `cat` command logic.
- `asm/*.asm` are the current flat-binary command images used by the kernel,
  including `demo`, `uptime`, `ls`, `cat`, and `whoami`.
- `elf/*.rs` contains early Rust `no_std` ELF command experiments.

For now the bootable command images are assembled with:

```sh
make userland-bin
```

The generated artifacts live in `target/userland/*.bin` and
`target/userland/*.elf` and are embedded into the kernel RAM filesystem.
Later milestones should replace the flat binary format with ELF output from
Rust userland command crates.
