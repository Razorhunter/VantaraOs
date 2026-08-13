# Vantara OS Kernel Roadmap

Roadmap ini susun kerja kernel ikut milestone kecil yang boleh diuji satu per satu.
Fokus utama: stabilkan foundation dulu, kemudian naikkan scheduler, driver, filesystem,
dan user mode.

## Milestone 0: Project Hygiene

- [ ] Betulkan ownership `target/` supaya `cargo check` biasa tidak fail.
- [x] Tambah `.gitignore` yang exclude `target/`, `build/`, boot images, dan QEMU artifacts.
- [x] Isi `README.md` dengan cara build, test, boot, dan run QEMU.
- [x] Tambah command ringkas dalam `Makefile`:
  - [x] `make check`
  - [x] `make test`
  - [x] `make run`
  - [x] `make clean`
- [x] Pastikan `cargo fmt` dan `cargo check --tests` jadi baseline wajib sebelum commit.

## Milestone 1: Boot And Diagnostics

- [x] Papar boot banner Vantara OS dengan version/build info.
- [x] Tambah serial logger level asas:
  - [x] `error`
  - [x] `warn`
  - [x] `info`
  - [x] `debug`
- [x] Tambah panic screen ringkas di VGA selain serial output.
- [x] Tambah kernel self-check semasa boot:
  - [x] GDT loaded
  - [x] IDT loaded
  - [x] PIC initialized
  - [x] heap initialized
  - [x] frame allocator usable
- [x] Tambah reboot/shutdown helper untuk QEMU.

## Milestone 2: Memory Management

- [x] Audit `BootInfoFrameAllocator` supaya tidak allocate frame yang sudah digunakan kernel.
- [x] Tambah physical frame allocation stats:
  - [x] total usable frames
  - [x] allocated frames
  - [x] remaining frames
- [x] Tambah page mapping API yang reusable:
  - [x] map page
  - [x] unmap page
  - [x] translate virtual address
  - [x] map range
- [x] Asingkan demo mapping `create_example_mapping` daripada production boot path.
- [x] Tambah heap diagnostics:
  - [x] heap start/end
  - [x] used bytes
  - [x] free bytes
  - [x] allocation failure count
- [x] Tambah tests untuk allocator edge cases:
  - [x] small allocations
  - [x] large allocations
  - [x] alignment-sensitive allocations
  - [x] allocation after free

## Milestone 3: Interrupts And Time

- [x] Tambah global tick counter daripada PIT timer interrupt.
- [x] Tambah sleep/delay API berdasarkan timer ticks.
- [x] Tambah IRQ enable/disable helper yang lebih jelas.
- [x] Pastikan semua interrupt handler acknowledge PIC walaupun driver path error.
- [x] Tambah keyboard/mouse event counters untuk debugging.
- [x] Tambah test atau boot smoke check untuk breakpoint interrupt.

## Milestone 4: Input Stack

- [x] Stabilkan keyboard driver:
  - [x] handle key press
  - [x] handle key release
  - [x] track modifiers: Shift, Ctrl, Alt, Fn, Super
  - [x] expose typed characters separately from raw keys
- [x] Stabilkan mouse driver:
  - [x] initialize PS/2 mouse hardware explicitly
  - [x] support packet resync
  - [x] support button release events
  - [x] clamp cursor to VGA dimensions
- [x] Tambah bounded input queue behavior:
  - [x] dropped event counter
  - [x] queue capacity stats
  - [x] non-blocking dequeue API
- [x] Tambah simple input debug overlay di VGA.

## Milestone 5: VGA Console And Shell

- [x] Pisahkan text console daripada mouse cursor renderer.
- [x] Simpan color code original cursor cell, bukan hanya character.
- [x] Tambah console API:
  - [x] clear screen
  - [x] set color
  - [x] write at row/column
  - [x] scroll
  - [x] backspace
- [x] Tambah command line buffer.
- [x] Bina kernel shell minimum:
  - [x] `help`
  - [x] `clear`
  - [x] `mem`
  - [x] `tasks`
  - [x] `uptime`
  - [x] `reboot`

## Milestone 6: Scheduler Foundation

- [x] Tukar `create_task` supaya terima typed entry point, bukan raw `u64`.
- [x] Pastikan stack task aligned kepada 16 bytes.
- [x] Tambah task lifecycle:
  - [x] Ready
  - [x] Running
  - [x] Blocked
  - [x] Sleeping
  - [x] Zombie
- [x] Tambah idle task.
- [x] Tambah cooperative yield API.
- [x] Sambungkan timer interrupt kepada scheduler tick.
- [x] Implement context switch pertama secara terkawal.
- [x] Tambah scheduler stats:
  - [x] current task
  - [x] ready queue length
  - [x] total context switches
  - [x] per-task runtime ticks
- [x] Tambah tests untuk ready queue dan task state transitions.

## Milestone 7: Kernel Synchronization

- [x] Review semua `spin::Mutex` yang digunakan dalam interrupt context.
- [x] Tambah interrupt-safe lock helper jika perlu.
- [x] Elakkan serial/VGA logging berat dalam hot interrupt path.
- [x] Tambah lock ordering notes untuk subsystem utama:
  - [x] input
  - [x] VGA
  - [x] scheduler
  - [x] allocator
- [x] Tambah guard untuk nested interrupt-sensitive sections.

## Milestone 8: Device Drivers

- [x] PS/2 controller init:
  - [x] detect controller
  - [x] enable keyboard port
  - [x] enable mouse port
  - [x] configure IRQs
- [x] UART driver:
  - [x] init port
  - [x] log levels
  - [ ] optional ring buffer
- [x] PCI discovery:
  - [x] scan bus/device/function
  - [x] list vendor/device/class
  - [x] expose device table
- [x] USB roadmap split:
  - [x] detect UHCI via PCI instead of hardcoded I/O base
  - [x] initialize controller only if found
  - [x] enumerate root ports
  - [x] parse descriptors
  - [x] support HID keyboard/mouse:
    - [x] Detect boot keyboard/mouse interface dan interrupt-IN endpoint daripada descriptor.
    - [x] Poll UHCI interrupt-IN dengan DATA toggle, interval, timeout, dan disconnect handling.
    - [x] Route keyboard/mouse boot reports kepada kernel input queue.
    - [x] Luluskan QEMU USB keyboard + mouse input regression.

## Milestone 9: Storage And Filesystem

- [x] Implement block device trait.
- [x] Tambah RAM disk untuk development.
- [ ] Tambah tar/initrd loader dari bootloader module jika sesuai.
- [x] Implement read-only filesystem minimum:
  - [x] list files
  - [x] read file
  - [x] stat file
- [x] Tambah shell commands:
  - [x] `ls`
  - [x] `cat`
  - [x] `stat`

## Milestone 10: User Mode Preparation

- [x] Audit GDT/TSS untuk user/kernel segments.
- [x] Tambah syscall entry plan:
  - [x] interrupt-based syscall
  - [x] syscall number ABI
  - [x] argument registers
  - [x] return values/errors
- [x] Tambah per-process address space design.
- [x] Tambah ELF loader research/prototype.
- [x] Tambah process model:
  - [x] PID
  - [x] parent PID
  - [x] address space
  - [x] file handles later

## Milestone 10.1: First Ring-3 User Task

- [x] Tambah user code/data descriptors dalam GDT.
- [x] Map user stack dengan permission user-accessible.
- [x] Siapkan transition kernel -> user mode via `iretq`.
- [x] Register syscall interrupt handler untuk `int 0x80`.
- [x] Embed first tiny user task binary atau hand-written user stub.
- [x] Tambah controlled shell command untuk boot first user task.
- [x] Implement `SYS_EXIT` checkout path untuk first user task.
- [x] Return from `SYS_EXIT` to kernel shell event loop.
- [x] Implement prototype `SYS_WRITE` untuk first user task.
- [x] Tambah Linux-like basic syscalls:
  - [x] `SYS_GETUID`
  - [x] `SYS_GETGID`
  - [x] `SYS_WHOAMI`
  - [x] `SYS_LISTDIR`
  - [x] `SYS_READ_FILE`
  - [x] `SYS_STAT`
- [x] Demo first user task panggil `SYS_UPTIME` dan print hasil via `SYS_WRITE`.
- [x] Demo first user task panggil `SYS_LISTDIR("/")` dan print hasil via `SYS_WRITE`.
- [x] Boot first user task dari shell dan return balik ke kernel shell.
- [x] Jalankan `/bin` user programs melalui generated registry:
  - [x] `demo`
  - [x] `uptime`
  - [x] `ls`
  - [x] `cat`
  - [x] `whoami`
- [x] Track process history untuk user programs:
  - [x] PID
  - [x] parent PID
  - [x] process name/path
  - [x] exit status

## Milestone 11: Testing And CI

- [x] Standardize QEMU test config.
- [x] Tambah boot smoke test yang check serial output.
- [ ] Tambah integration tests untuk:
  - [x] heap allocation
  - [x] page mapping
  - [x] keyboard scancode decode
  - [x] input queue
  - [x] scheduler queue
- [x] Tambah GitHub Actions atau local CI script.
- [x] Simpan known QEMU command examples dalam docs.

## Milestone 12: Documentation

- [ ] Tulis `docs/architecture.md`.
- [ ] Tulis `docs/boot-flow.md`.
- [ ] Tulis `docs/memory.md`.
- [ ] Tulis `docs/interrupts.md`.
- [ ] Tulis `docs/scheduler.md`.
- [ ] Tulis `docs/drivers.md`.
- [ ] Tambah decision log untuk pilihan besar seperti allocator, scheduler, dan syscall ABI.

## Milestone 13: ELF Userland Loader

- [x] Parse ELF64 executable header untuk x86_64.
- [x] Parse ELF64 program headers.
- [x] Validate `PT_LOAD` segment permissions dan memory range.
- [x] Tambah helper untuk map ELF load segments ke user pages.
- [x] Zero-fill `.bss` area apabila `mem_size > file_size`.
- [x] Set Ring-3 entry point dari ELF header, bukan fixed `0x400000`.
- [x] Build satu Rust `no_std` user command sebagai ELF.
- [x] Embed `.elf` artifacts dalam `/bin` registry.
- [x] Tambah loader dispatch untuk mixed `/bin`: flat legacy binaries dan ELF binaries.
- [x] Integrate ELF segment mapping dengan runtime exec path.

## Milestone 14: Process Isolation And File Descriptors

- [x] Simpan entry/stack sebenar bagi setiap process execution.
- [x] Beri setiap user process user memory layout sendiri:
  - [x] Simpan layout descriptor per process.
  - [x] Assign stack range per process.
  - [x] Clear program area dan stack slot sebelum exec.
  - [x] Enforce syscall buffers kepada layout process semasa.
  - [x] Pisahkan address space descriptor daripada process table.
  - [x] Reserve private P4 frame pool untuk user address spaces.
  - [x] Populate prepared P4 frames dengan snapshot active page table.
  - [x] Track actual program footprint dalam per-process layout.
  - [x] Add controlled CR3 smoke switch kepada prepared P4 snapshot.
  - [x] Verify assigned process P4 frame sebelum user jump.
  - [x] Run user program under assigned prepared P4 and restore kernel P4 on exit.
  - [x] Remap initial user program pages and stack page to private process frames.
  - [x] Size private program page pool from bundled userland images.
  - [x] Add multi-page user stacks with guard pages.
  - [x] Report user page faults with pid/name/address/rip.
  - [x] Promote verified private mappings to `IsolatedUser` process status.
  - [x] Tambah `/bin/fault` untuk test user fault handling.
  - [x] Asingkan initial program mapping per process melalui private frames.
  - [x] Tukar user execution ke process page table dan restore kernel P4 on exit.
  - [x] Tambah lifecycle allocator/free untuk user address-space frames.
- [x] Tambah process-owned user stack slot mapping.
- [x] Tambah file descriptor table minimum:
  - [x] stdin
  - [x] stdout
  - [x] stderr
  - [x] open/read/write/close
- [x] Tukar `cat` kepada fd-based syscall path.
- [x] Tambah directory handle ringkas melalui fd-backed listing buffer.
- [x] Tukar `ls` kepada fd-based syscall path.

## Milestone 15: Userland Init And Shell

- [x] Tambah `/bin/init` sebagai persistent userland service manager.
- [x] Tambah manual/boot policy untuk launch `/bin/init`.
- [x] Boot kernel -> `/bin/init`.
- [x] Tambah syscall `SYS_EXEC` prototype untuk chain userland program.
- [x] Jadikan `/bin/sh` sebagai userland shell prototype.
- [x] Tambah prototype `read(STDIN)` untuk userland shell.
- [x] Auto-respawn `/bin/sh` selepas command userland tamat.
- [x] Tambah `SYS_WAIT` marker untuk shell wait-style bridge.
- [x] Tambah login prototype.
- [x] Pindahkan command built-in yang sesuai keluar dari kernel shell.

## Milestone 16: Real Process Control

- [x] Reserve PID semasa `SYS_EXEC` supaya caller dapat child PID sebelum program dijalankan.
- [x] Simpan parent PID sebenar untuk process yang dilancarkan dari userland.
- [x] Tukar `/bin/sh` supaya `wait` hantar target child PID.
- [x] Validate wait target terhadap pending child process.
- [x] Mark exited child process sebagai `Zombie` sehingga parent collect status.
- [x] Reap waited child process sebelum respawn `/bin/sh` bridge.
- [x] Tambah `SYS_PROCS` dan `/bin/procs` untuk inspect process table dari userland.
- [x] Tambah kernel fallback reaper untuk zombie yang parent-nya sudah tiada.
- [x] Track `waiting_for` PID semasa `/bin/sh` panggil waitpid bridge.
- [x] Capture user `RIP/RSP/RFLAGS` semasa waitpid sebagai resume context prototype.
- [x] Tukar `SYS_WAIT` supaya parent shell yield ke kernel scheduler, bukan return terus.
- [x] Implement blocking `waitpid(pid)` sebenar tanpa perlu respawn `/bin/sh`.
- [x] Return child exit status terus kepada parent shell.
- [x] Tambah process table view yang bezakan running/exited/zombie/reaped.

## Milestone 17: Process Robustness

- [x] Tambah process table compaction untuk hadkan rekod `Reaped` lama.
- [x] Handle `waitpid(pid)` untuk child yang sudah `Zombie` sebelum parent sempat block.
- [x] Return error wait yang lebih jelas untuk non-child atau PID tidak wujud.
- [x] Tambah exit-status inspection yang lebih clean dalam `/bin/procs`.
- [x] Tambah process lifecycle invariant tests:
  - [x] parent wait child running
  - [x] parent wait child zombie
  - [x] invalid wait target
  - [x] orphan zombie reaped by init
- [x] Tambah `/bin/kill` atau signal placeholder untuk process control berikutnya.
- [x] Kurangkan kernel debug output yang bocor ke VGA untuk userland command normal.

## Milestone 18: Memory Protection Hardening

- [x] Audit semua syscall yang terima user pointer:
  - [x] null pointer
  - [x] pointer luar user memory range
  - [x] range overflow
  - [x] buffer cross-page
- [x] Bezakan user read pointer dan user write pointer validation.
- [x] Tambah centralized `UserPtr`/`UserSlice` helper untuk syscall.
- [x] Pastikan userland tidak boleh read/write kernel memory.
- [x] Jadikan guard page stack fault report lebih jelas.
- [x] Tambah `/bin/fault` scenario tambahan:
  - [x] invalid read
  - [x] invalid write
  - [x] stack guard fault
  - [x] bad syscall pointer
- [x] Pastikan user page fault kill process sahaja, bukan panic/reboot kernel.
- [x] Tambah tests untuk pointer validation dan page fault classification.
  - [x] pointer validation
  - [x] page fault classification

## Milestone 19: Scheduler For Real User Tasks

- [x] Pisahkan scheduler task kernel dan process user dalam model yang lebih jelas.
- [x] Tambah ready queue untuk user process.
- [x] Support lebih dari satu user process Ready/Running lifecycle.
  - [x] Support lebih dari satu process `Ready` dalam FIFO queue.
  - [x] Support pending user program queue supaya request baru tidak overwrite request lama.
  - [x] Support cooperative switch antara shell dan background user process.
  - [x] Support context switch cooperative antara lebih dari satu runnable user process.
- [x] Tambah `yield` syscall prototype untuk user process.
- [x] Tambah timer-driven preemption prototype untuk user process.
  - [x] Tambah timer preemption check counter dari PIT interrupt.
  - [x] Timer interrupt boleh request user task yield/reschedule.
  - [x] Consume pending timer preempt request ketika user process yield.
- [x] Simpan full user register context untuk resume, bukan hanya `RIP/RSP/RFLAGS`.
- [x] Context switch antara user process tanpa perlu balik ke kernel shell.
  - [x] Cooperative `yield` boleh keluar ke kernel scheduler dan resume semula dari ready queue.
  - [x] Shell idle keyboard yield supaya background process boleh sambung jalan.
  - [x] Switch antara dua process user berbeza tanpa respawn shell.
  - [x] Tambah test FIFO resume untuk dua yielded user process.
- [x] Tambah `/bin/sleep` atau `/bin/yielddemo` untuk test scheduling.
  - [x] Tambah `/bin/sleep <ms>` cooperative sleep test.
- [x] Tambah `/bin/sh` command `bg <cmd>` untuk queue kerja background prototype.
- [x] Tambah background zombie reaper untuk cleanup job yang tidak di-`wait`.
  - [x] Bezakan foreground/background job secara explicit dalam process metadata.
- [x] Jalankan process maintenance sebelum scheduler handoff supaya orphan zombie tidak tertinggal apabila ada pending user program.
- [x] Tambah `SYS_READ` stdin `WouldBlock` supaya shell/login tidak anggap empty read sebagai data.
- [x] Tambah `/bin/procs` view untuk runtime tick/context switch count.
  - [x] Papar `yield` syscall count.
  - [x] Papar user context-switch metric awal.
  - [x] Papar pending program queue count.
  - [x] Papar timer preemption check count.
  - [x] Papar runtime tick/context switch count sebenar.
  - [x] Papar kernel/user process count dan user Ready/Running/Blocked metrics.
  - [x] Papar foreground/background user process count.
  - [x] Papar ready queue position per process.
  - [x] Papar ready queue skip count untuk stale/non-resumable entries.

## Milestone 20: Userland ABI And Libc-Lite

- [x] Stabilkan syscall ABI document:
  - [x] syscall number
  - [x] argument registers
  - [x] return/error convention
  - [x] clobbered registers
- [x] Pindahkan syscall wrapper common ke shared userland crate.
- [x] Tambah libc-lite helper:
  - [x] `write`
  - [x] `read`
  - [x] `open`
  - [x] `close`
  - [x] `exec`
  - [x] `waitpid`
  - [x] `exit`
- [x] Bina command userland baru atas helper common, bukan inline asm per binary.
  - [x] Port `/bin/rusthello` kepada shared ABI wrapper.
  - [x] Port `/bin/procs` kepada shared ABI wrapper.
  - [x] Port `/bin/stat` kepada shared ABI wrapper.
  - [x] Port `/bin/kill` kepada shared ABI wrapper.
  - [x] Port `/bin/login` kepada shared ABI wrapper.
  - [x] Port `/bin/sh` kepada shared ABI wrapper.
  - [x] Port `/bin/yielddemo` kepada shared ABI wrapper.
  - [x] Port `/bin/fault` kepada shared ABI wrapper untuk path biasa, dengan raw fault syscall khusus dikekalkan untuk ujian pointer invalid.
- [x] Tambah simple argv-lite handoff untuk process baru.
  - [x] Dokumentasi fixed arg area sementara.
  - [x] Shared ABI helper `program_arg`, `argv`, `first_arg`.
  - [x] Port `/bin/sleep` kepada shared ABI argv helper.
- [x] Tambah `SYS_SLEEP_MS` supaya `/bin/sleep` block di kernel, bukan busy-loop.
- [x] Tambah full argc/argv/envp initial stack handoff.
  - [x] Bina initial stack image dengan `argc`, `argv[]`, null `envp`, dan `AT_NULL`.
  - [x] Seed initial stack untuk shared user mapping.
  - [x] Seed initial stack untuk isolated/private P4 process.
  - [x] Kekalkan fixed arg slot lama untuk compatibility command sedia ada.

## Milestone 21: Filesystem Evolution

- [x] Tambah path normalization yang lebih lengkap:
  - [x] `.`
  - [x] `..`
  - [x] repeated `/`
  - [x] relative path dari working directory/helper cwd
- [x] Tambah per-process current working directory.
- [x] Tambah `SYS_CHDIR` dan `/bin/pwd`.
- [x] Tambah `/bin/cd` handling dalam shell.
- [x] Tambah metadata basic:
  - [x] file type
  - [x] readonly flag
  - [x] size
  - [x] inode/id placeholder
- [x] Rancang writable RAM filesystem:
  - [x] create file
  - [x] write file
  - [x] delete file
  - [x] rename file
  - [x] Dokumentasi design di `docs/filesystem-evolution.md`.

## Milestone 22: Shell And Session Polish

- [x] Kurangkan kernel debug output di VGA, jadikan input debug opt-in.
- [x] Tambah shell command history minimum.
- [x] Tambah line editing:
  - [x] left/right cursor
  - [x] home/end
  - [x] clear line
  - [x] delete/backspace middle-line redraw
  - [x] visual blinking text cursor
- [x] Tambah shell built-in:
  - [x] `cd`
  - [x] `pwd`
  - [x] `exit`
  - [x] `help`
  - [x] `reboot`
- [x] Tambah prompt yang tunjuk user/cwd.
- [x] Login/session state lebih jelas:
  - [x] uid/gid per process
  - [x] session owner
  - [x] logout flow

## Milestone 23: Device And Driver Maturity [COMPLETE]

- [x] UART optional ring buffer.
- [x] PCI device registry yang boleh query dari userland.
- [x] USB descriptor parsing.
- [x] USB HID keyboard/mouse roadmap implementation.
- [x] Network device detection skeleton.
- [x] Driver error reporting yang tidak spam VGA.

## Milestone 24: Packaging, Tests, And Release Discipline [COMPLETE]

- [x] Tambah boot integration test untuk `/bin/init -> login -> sh`.
- [x] Tambah serial golden-output smoke tests untuk command userland utama:
  - [x] `ls`
  - [x] `cat`
  - [x] `procs`
  - [x] `rusthello`
- [x] Tambah regression tests untuk page fault dan process isolation.
- [x] Tambah build artifact manifest:
  - [x] kernel image
  - [x] userland binaries
  - [x] generated registry
- [x] Tambah version/build metadata yang include git/build timestamp jika available.
- [x] Tulis release checklist untuk Vantara OS dev image.

## Mainstream-Capable OS Track

Track ini bukan milestone kecil; ini arah besar supaya Vantara bergerak daripada mini OS
kepada platform OS yang boleh dibandingkan dengan kernel mainstream secara berperingkat.

### Phase A: Kernel Correctness And Safety [COMPLETE]

- [x] Formalize syscall ABI dan compatibility policy.
- [x] Harden memory isolation untuk semua user pointer dan page fault path.
- [x] Tambah kernel/user fault containment supaya user process tidak boleh panic kernel.
- [x] Tambah regression suite untuk process, memory, syscall, filesystem, dan boot.
- [x] Audit unsafe Rust blocks dengan safety notes.

### Phase B: Real Multitasking Platform [COMPLETE]

- [x] Preemptive scheduler untuk user process.
  - [x] Custom PIT entry yang simpan full Ring-3 register/interrupt frame.
  - [x] Timer quantum boleh preempt process semasa dan resume ready process lain.
  - [x] Preserve `RAX` dan CR3 merentas timer-driven context switch.
  - [x] Tambah compile-time frame-layout assertions dan scheduler regression test.
  - [x] Tambah QEMU integration test untuk CPU-bound process tanpa cooperative yield.
  - [x] Luluskan QEMU preemption integration test dalam Docker dengan QEMU tersedia.
- [x] Full register context save/restore.
- [x] Thread model awal:
  - [x] process
  - [x] thread
    - [x] Main thread identity per process dengan allocator TID berasingan.
    - [x] Sinkronkan lifecycle process/main-thread dan scheduler handoff.
    - [x] Pindahkan saved context/runtime accounting daripada process ke thread.
    - [x] Tukar user ready queue kepada explicit TID queue.
    - [x] Tambah per-process thread store dan lookup global berdasarkan TID.
    - [x] Kira serta papar jumlah thread sebenar dalam `/bin/procs`.
    - [x] Support lebih daripada satu thread dalam process yang sama.
      - [x] Model boleh mendaftarkan lebih daripada satu thread yang berkongsi process.
      - [x] Validate user-supplied stack dan bina initial context secondary thread.
      - [x] Jadualkan secondary thread secara live melalui `SYS_THREAD_CREATE`.
      - [x] Tambah `SYS_THREAD_EXIT` tanpa menamatkan seluruh process.
      - [x] Track current TID merentas cooperative yield dan timer preemption.
      - [x] Tambah `/bin/threaddemo` QEMU integration smoke.
      - [x] Tambah blocking `SYS_THREAD_JOIN` dengan wake-on-thread-exit.
      - [x] Kernel-owned stack allocation.
        - [x] Sediakan private stack frames per slot tanpa alias shared mapping.
        - [x] Reserve/clear/reuse stack slot melalui `SYS_THREAD_SPAWN`.
        - [x] Kekalkan `SYS_THREAD_CREATE` user-supplied stack untuk ABI compatibility.
  - [x] kernel task
    - [x] Typed kernel-task context dan first-task bootstrap.
    - [x] Buang context-switch entry lama yang tidak selamat/tidak digunakan.
    - [x] Satukan identity/lifecycle kernel task ke process `Thread` sebenar.
    - [x] Gunakan global TID allocator dan TID ready queue untuk kernel task.
    - [x] Jadikan process table authority untuk state/runtime kernel thread.
    - [x] Aktifkan live timer-driven context switching antara kernel threads.
      - [x] Bezakan fresh-entry frame daripada saved CPU interrupt frame.
      - [x] Tambah QEMU regression yang membuktikan A/B/C dipreempt dan disambung semula.
    - [x] Tambah preemption-disable guard dan audit semua lock sebelum enable secara default.
      - [x] Tambah nested `PreemptionGuard` dengan deferred timer preemption.
      - [x] Ganti kernel mutex dengan `PreemptMutex` yang IRQ/preemption-safe.
      - [x] Tambah QEMU proof bahawa critical section tidak dipreempt.
    - [x] Integrasikan kernel-thread scheduler dalam normal boot lifecycle.
      - [x] Jalankan runtime event loop pada scheduler-owned kernel stack.
      - [x] Kekalkan A/B/C sebagai feature-only regression workload.
      - [x] Tambah normal QEMU smoke markers untuk scheduler bootstrap.
- [x] IPC primitive awal:
  - [x] pipe
    - [x] Process-owned read/write descriptor pair.
    - [x] Bounded FIFO, non-blocking `WouldBlock`, close dan EOF semantics.
    - [x] Tambah `/bin/pipedemo` thread-to-thread QEMU regression.
  - [x] event/wait queue
    - [x] Process-owned auto-reset event handle.
    - [x] FIFO waiter TID queue dengan saved syscall context.
    - [x] Block/wake hanya thread pemanggil melalui process-table authority.
    - [x] Latched signal, close guard, unit tests dan `/bin/eventdemo` regression.
  - [x] message queue placeholder
    - [x] Process-owned bounded queue dan fixed message boundary.
    - [x] Non-blocking empty/full `WouldBlock` semantics.
    - [x] Reject undersized receive buffer tanpa consume message.
    - [x] Unit tests dan `/bin/msgdemo` thread-to-thread QEMU regression.
- [x] Signal/job control model yang lebih lengkap.
  - [x] Baseline signal/job control:
    - [x] Implement `SIGTERM` delivery melalui syscall `kill`.
    - [x] Enforce root/same-UID permission dan lindungi kernel task.
    - [x] Gunakan conventional signal exit status `128 + signal`.
    - [x] Cleanup process-owned FD/IPC/address space semasa termination.
    - [x] Bangunkan parent yang blocked dalam `waitpid`.
    - [x] Tambah process-table invariant tests dan QEMU background-job regression.
  - [x] Advanced signal/job control:
    - [x] Pending signal set dan blocked signal mask.
      - [x] Process-wide signal bitset dengan `SIG_BLOCK`, `SIG_UNBLOCK`, dan `SIG_SETMASK`.
      - [x] Queue blocked `SIGTERM` dan expose pending set kepada userland.
      - [x] Deliver default action apabila pending `SIGTERM` dinyahsekat.
      - [x] Tambah `/bin/signaldemo`, invariant tests, dan QEMU regression.
    - [x] User-space signal handlers dan default dispositions.
      - [x] Tambah `SIG_DFL`, `SIG_IGN`, dan executable handler disposition.
      - [x] Simpan interrupted context dalam protected per-thread kernel metadata.
      - [x] Tambah handler dispatch, nested-delivery deferral, dan `sigreturn`.
      - [x] Extend `/bin/signaldemo`, invariant tests, dan QEMU regression.
    - [x] Process groups/sessions untuk kawalan job.
      - [x] Tambah inherited `PGID`/`SID` pada setiap process.
      - [x] Implement `getpgrp`, `setpgid`, `getsid`, dan `setsid`.
      - [x] Enforce same-session, child-target, group-leader, dan session-leader guards.
      - [x] Papar `PGID`/`SID` dalam `/bin/procs`.
      - [x] Tambah `/bin/jobdemo`, invariant tests, dan QEMU command regression.
    - [x] Terminal-generated signals seperti `SIGINT` dan `SIGTSTP`.
      - [x] Detect `Ctrl-C`/`Ctrl-Z` pada PS/2 dan USB keyboard tanpa bocor ke stdin.
      - [x] Queue terminal control event dari IRQ dan deliver dalam runtime context.
      - [x] Track foreground job semasa shell blocked dalam `waitpid`.
      - [x] Default `SIGINT` terminate foreground job dan pulihkan shell.
      - [x] Tambah `Stopped` lifecycle untuk `SIGTSTP`.
      - [x] Tambah `SIGCONT` melalui `kill <pid> 18`.
      - [x] Tambah invariant tests dan QEMU terminal-signal regression.

#### Correctness Consolidation Sprint

- [x] Harden PIT entry untuk frame Ring-0/Ring-3 dan SysV stack alignment.
- [x] Buang kernel `switch_context` lama yang corrupt register.
- [x] Betulkan full-register `setup_first_task` bootstrap.
- [x] Enforce ELF W^X, NX stack/data, dan reject writable+executable segment.
- [x] Page-align userland text/rodata/data melalui linker script.
- [x] Lulus full Docker/QEMU regression selepas hardening.

### Phase C: Persistent System Foundation [COMPLETE]

- [x] Writable filesystem dengan metadata asas dan directory tree sebenar.
  - [x] Tambah mutable inode store dan `/tmp` directory node.
  - [x] Sambungkan live inode read/write kepada process-owned file descriptor.
  - [x] Implement create, write, unlink, dan rename untuk direct child `/tmp`.
  - [x] Tambah `/bin/touch`, `/bin/write`, `/bin/rm`, dan `/bin/mv`.
  - [x] Tambah VANTFS block-backed filesystem pada `/persist`.
  - [x] Tambah ATA PIO polling path untuk dedicated QEMU persistence disk.
  - [x] Buktikan data survive dua boot QEMU menggunakan disk image sama.
  - [x] Tambah nested mutable directories.
  - [x] Tambah `mkdir`/`rmdir` dengan empty-directory guard.
  - [x] Kekalkan compatibility entry VANTFS lama sebagai root file.
  - [x] Defer metadata lanjutan ke filesystem hardening selepas package foundation.
- [x] VFS layer supaya filesystem backend boleh ditukar.
  - [x] Tetapkan root layout Vantara: `/System`, `/Apps`, `/Users`, `/Config`, `/Data`, `/Cache`, `/Logs`, `/Runtime`, `/Devices`, `/Temp`, `/Packages`, `/Boot`, dan `/Volumes`.
  - [x] Jadikan `/Data` dan `/Devices` mount canonical sambil kekalkan `/persist` dan `/dev` sebagai compatibility aliases.
  - [x] Tambah object-safe filesystem backend contract.
  - [x] Tambah mount table dan longest-prefix path resolver.
  - [x] Gunakan `(mount_id, inode)` sebagai identiti node global.
  - [x] Pindahkan RAM filesystem ke backend berasingan.
  - [x] Reject rename merentas backend dengan cross-device error.
- [x] Init/service manager yang boleh spawn dan monitor services.
  - [x] Reserve PID 0 untuk kernel task dan PID 1 sebenar untuk `/bin/init`.
  - [x] Jalankan `/bin/init` tanpa parent sebagai root session leader.
  - [x] Supervise `/bin/login` menggunakan blocking `waitpid`.
  - [x] Restart login selepas exit dengan restart backoff.
  - [x] Kekalkan zombie child PID 1 sehingga dikutip oleh service manager.
  - [x] Tambah QEMU regression yang terminate login dan sahkan respawn.
- [x] Device namespace:
  - [x] Mount `devfs` pada `/dev` melalui VFS.
  - [x] Expose `/dev/null` dan `/dev/zero` dengan device read/write semantics.
  - [x] Gunakan driver status table sebagai kernel driver registry.
  - [x] Expose live `/dev/drivers`, `/dev/pci`, dan `/dev/net`.
  - [x] Tambah QEMU regression untuk device nodes dan registry views.
- [x] Package/build layout untuk kernel + userland + initrd.
  - [x] Bina canonical initrd daripada generated userland registry.
  - [x] Tambah deterministic initrd dan package manifests dengan SHA-256.
  - [x] Package kernel image, initrd, blank persistence disk, dan metadata.
  - [x] Tambah `make initrd`, `make package`, dan `make package-test`.
  - [x] Buktikan rebuild dengan `SOURCE_DATE_EPOCH` sama menghasilkan hash sama.

Deferred filesystem hardening:

- [ ] Tambah timestamp, ownership, permissions, dan link count.
- [ ] Enforce access mode dan ownership dalam VFS/syscall path.

### Phase D: Hardware And I/O Maturity

- [x] Storage driver path:
  - [x] ATA PIO polling baseline untuk persistence disk.
  - [x] AHCI/NVMe roadmap
    - [x] Detect PCI AHCI class `01/06/01`.
    - [x] Decode 32-bit/64-bit PCI BAR dan validate ABAR5 memory BAR.
    - [x] Tambah AHCI controller registry dan `/dev/ahci`.
    - [x] Tambah Q35 QEMU discovery regression.
    - [x] Tambah audited PCI MMIO mapper untuk ABAR.
    - [x] Map ABAR uncached, write-through, writable, dan non-executable.
    - [x] Parse HBA capabilities/version/implemented ports.
    - [x] Expose CAP/PI/VS melalui `/dev/ahci`.
    - [x] Allocate page-aligned command-list, received-FIS, command-table, dan data frames.
    - [x] Stop/rebase/start first active SATA port dengan PxCMD timeout guards.
    - [x] Initialize slot-0 command header dan one-entry 512-byte PRDT.
    - [x] Implement polled AHCI IDENTIFY DEVICE dan parse model disk.
    - [x] Implement dan validate single-sector AHCI `READ DMA EXT`.
    - [x] Expose synchronized read-only AHCI `BlockDevice` dengan capacity daripada IDENTIFY.
    - [x] Implement AHCI `WRITE DMA EXT` dan `FLUSH CACHE EXT` dalam `BlockDevice`.
    - [x] Buktikan AHCI write + flush persistence merentas dua boot QEMU pada image terasing.
    - [x] Buktikan MBR -> cache -> VANTFS melalui AHCI merentas dua boot QEMU.
    - [x] Tambah controlled strict `storage-ahci` backend selection.
    - [x] Jalankan `/persist` shell lifecycle melalui AHCI disk kedua merentas dua boot.
    - [x] Tambah `storage-auto`: prefer AHCI disk kedua, fallback ATA PIO dengan diagnostics.
    - [x] Buktikan preferred-AHCI dan fallback-ATA masing-masing persist merentas dua boot.
    - [x] Promote tested `storage-auto` policy sebagai default persistence selection.
    - [x] Kekalkan strict `storage-ata` dan `storage-ahci` recovery builds.
    - [x] Detect PCI NVMe class `01/08/02` dan decode BAR0.
    - [x] Map NVMe BAR0 melalui audited uncached MMIO window.
    - [x] Parse CAP/VS/CC/CSTS, queue limit, doorbell stride, dan page-size range.
    - [x] Tambah NVMe registry, `/dev/nvme`, dan QEMU discovery regression.
    - [x] Implement guarded CC.EN/CSTS.RDY controller ownership transition.
    - [x] Allocate private 64-entry admin SQ/CQ dan program AQA/ASQ/ACQ.
    - [x] Submit polled NVMe Identify Controller dan parse model/serial/NN.
    - [x] Submit Identify Namespace NSID 1 dan parse capacity/LBA format.
    - [x] Request queue resources dan create private 64-entry NVMe I/O CQ/SQ QID 1.
    - [x] Implement polled NVM single-block read dan validate LBA 0 payload.
    - [x] Implement NVMe write + flush dan buktikan persistence merentas dua boot QEMU.
    - [x] Expose synchronized NVMe `BlockDevice` dengan write-through flush semantics.
    - [x] Tambah NVMe kepada controlled `storage-auto` policy dan buktikan VANTFS persistence.
    - [x] Kekalkan strict `storage-nvme` recovery/diagnostic build.
  - [x] block cache
    - [x] Tambah bounded 16-sector LRU cache di atas `BlockDevice`.
    - [x] Gunakan write-through policy supaya persistence semantics kekal selamat.
    - [x] Sambungkan VANTFS metadata dan data I/O kepada cache.
    - [x] Expose hit/miss/write/eviction counters melalui `/dev/block-cache`.
    - [x] Tambah unit tests dan QEMU cache-hit regression.
  - [x] partition parser
    - [x] Parse dan validate empat primary MBR entries.
    - [x] Prefer Vantara partition type `0x7f`, fallback kepada first valid partition.
    - [x] Tambah bounded partition block view sebelum cache/VANTFS.
    - [x] Kekalkan superfloppy compatibility untuk persistence image lama.
    - [x] Expose active storage range melalui `/dev/partitions`.
    - [x] Buktikan partition-offset persistence merentas dua boot QEMU.
- [x] Network stack roadmap:
  - [x] NIC driver
    - [x] Detect Intel 82540EM/e1000 dan 82574L/e1000e PCI IDs.
    - [x] Decode dan map BAR0 MMIO melalui audited uncached window.
    - [x] Read CTRL/STATUS, link state, dan valid receive MAC address.
    - [x] Expose mapped NIC diagnostics melalui `/Devices/net` dan QEMU regression.
    - [x] Initialize 16-entry RX/TX descriptor rings dan dedicated packet buffers.
    - [x] Submit dan reclaim Ethernet frame pertama melalui TX ring.
    - [x] Poll, validate, dan recycle received Ethernet descriptors.
  - [x] Ethernet
    - [x] Encode minimum padded Ethernet II frame untuk TX.
    - [x] Parse destination/source MAC, EtherType, dan payload daripada RX DMA.
    - [x] Dispatch IPv4, ARP, Vantara-test, dan unknown EtherTypes.
    - [x] Reject malformed short frames dan oversized TX payloads.
  - [x] ARP
    - [x] Encode dan parse Ethernet/IPv4 ARP request/reply packets.
    - [x] Belajar pemetaan IPv4-to-MAC melalui bounded per-interface cache.
    - [x] Balas request untuk alamat IPv4 interface sendiri secara unicast.
    - [x] Buktikan request, reply, dan cache resolution melalui dua NIC QEMU.
  - [x] IPv4
    - [x] Encode dan parse minimum IPv4 header tanpa options.
    - [x] Generate dan validate Internet header checksum.
    - [x] Validate version, IHL, total length, destination, dan protocol dispatch.
    - [x] Reject malformed, oversized, dan fragmented packets buat masa ini.
    - [x] Buktikan unicast IPv4 delivery selepas ARP resolution melalui dua NIC QEMU.
  - [x] Transport
    - [x] UDP
      - [x] Encode dan parse source port, destination port, length, dan payload.
      - [x] Generate dan validate UDP checksum menggunakan IPv4 pseudo-header.
      - [x] Reject malformed length, oversized payload, dan invalid checksum.
      - [x] Buktikan datagram unicast port 40000 ke 7777 melalui dua NIC QEMU.
      - [x] Tambah bounded kernel socket table dengan bind, receive queue, dan close.
      - [x] Dispatch RX datagram kepada socket berdasarkan destination port.
      - [x] Reject duplicate bind dan drop secara terkawal apabila queue penuh.
      - [x] Tambah kernel UDP send API yang guna bound source port, ARP cache, IPv4, dan Ethernet TX path.
      - [x] Expose user-facing UDP syscalls untuk bind, send_to, recv_from, dan close.
      - [x] Tambah `/bin/udpdemo` command smoke untuk validate UDP syscall ABI surface.
      - [x] Poll RX semasa runtime dan buktikan live UDP delivery kepada proses userland.
    - [x] TCP
      - [x] Encode dan parse minimum TCP header, sequence/ack numbers, flags, window, dan payload.
      - [x] Generate dan validate TCP checksum menggunakan IPv4 pseudo-header.
      - [x] Reject malformed header length, invalid port, oversized payload, dan invalid checksum.
      - [x] Implement bounded connection table dan state machine SYN/SYN-ACK/ACK.
      - [x] Buktikan TCP handshake melalui dua NIC QEMU.
      - [x] Hantar TCP payload selepas established dan validate cumulative ACK.
      - [x] Implement passive FIN close dan buktikan close melalui dua NIC QEMU.
      - [x] Tambah kernel TCP socket handles untuk listen, non-blocking accept/receive, send, dan close.
      - [x] Tambah active connect dengan SYN-SENT ke ESTABLISHED transition.
      - [x] Expose TCP listen, accept, connect, send, receive, dan close syscalls kepada userland.
      - [x] Tambah `/bin/tcpdemo` dua-NIC smoke untuk validate TCP syscall transfer end-to-end.
      - [x] Enforce UDP/TCP socket-handle ownership mengikut PID pada syscall boundary.
      - [x] Reclaim semua socket milik proses secara automatik apabila proses exit.
      - [x] Add duplicate/out-of-order TCP sequence handling dan bounded receive-queue backpressure counters.
      - [x] Add bounded SYN/SYN-ACK retransmission dan timeout cleanup untuk half-open connections.
- [x] USB read-only baseline:
  - [x] USB HID keyboard/mouse baseline.
  - [x] UHCI transaction engine:
    - [x] Enable PCI I/O-space dan bus mastering untuk UHCI.
    - [x] Allocate DMA32 frame list, queue head, TD arena, dan data page.
    - [x] Program valid idle 1024-entry UHCI schedule sebelum controller run.
    - [x] Implement bounded synchronous TD-chain submission, data-toggle tracking, dan completion/error decode.
    - [x] Reset port dan enumerate device melalui endpoint-zero control transfers (`GET_DESCRIPTOR`, `SET_ADDRESS`, `SET_CONFIGURATION`).
  - [x] USB mass storage:
    - [x] Detect mass-storage interface class `08/06/50` dan pasangan endpoint Bulk IN/OUT.
    - [x] Implement Bulk-Only Transport CBW/data/CSW transaction baseline dengan tag dan residue validation.
    - [x] Implement SCSI INQUIRY dan TEST UNIT READY di atas Bulk-Only Transport.
    - [x] Implement READ CAPACITY(10) dan bounded READ(10) read-only path.
    - [x] Expose read-only USB mass-storage `BlockDevice` adapter dan `/dev/usb-storage` diagnostics.
    - [x] Tambah QEMU USB-storage discovery/read regression:
      - [x] Sediakan isolated UHCI + USB-storage image test untuk enumeration, capacity, dan LBA 0 marker.
      - [x] Luluskan regression dalam Docker/QEMU dengan UHCI enumeration, SCSI capacity, dan LBA 0 read.
- [x] Advanced USB maturity:
  - [x] Implement SCSI `REQUEST SENSE`, `WRITE(10)`, dan `SYNCHRONIZE CACHE(10)` primitives.
  - [x] Sediakan opt-in writable `BlockDevice` dengan flush selepas setiap sector write.
  - [x] Tambah dan luluskan QEMU USB write + flush persistence regression merentas reboot.
  - [x] Implement Bulk-Only reset recovery, clear Bulk IN/OUT endpoint stall, dan reset DATA toggle selepas phase/transfer failure.
  - [x] Tambah close/eject lifecycle, runtime port polling, handle invalidation, dan luluskan safe hot-unplug QEMU regression.
  - [x] Tambah EHCI/xHCI untuk USB 2.0/3.x dan hardware moden:
    - [x] Detect/map EHCI USB 2.0 capability/operational registers dengan `/dev/ehci` diagnostics dan QEMU discovery regression.
    - [x] Implement EHCI asynchronous schedule/QTD transfers dan companion-port handoff.
    - [x] Tambah xHCI USB 3.x controller dan transfer-ring path:
      - [x] Detect/map xHCI capability, operational, runtime dan doorbell registers.
      - [x] Initialize DCBAA, command ring, event ring, dan baseline transfer ring.
      - [x] Tambah `/dev/xhci` diagnostics dan QEMU xHCI ring bring-up regression.
    - [x] Enumerate high-speed USB device melalui EHCI qTD dan configure endpoint-zero sebenar.
    - [x] Implement xHCI command/device lifecycle:
      - [x] Implement command-ring doorbell dan polled command-completion event consumer.
      - [x] Implement `Enable Slot` dan `Address Device` dengan input/output context.
      - [x] Fetch descriptors melalui endpoint zero dan implement `Configure Endpoint`.
    - [x] Validate data transfer end-to-end melalui EHCI dan xHCI dalam QEMU:
      - [x] EHCI Bulk-Only/SCSI `INQUIRY` melalui Bulk OUT/IN qTD dan CSW validation.
      - [x] xHCI keyboard interrupt-IN Normal TRB hingga kernel input queue.
- [x] Graphics Baseline [COMPLETE]:
  - [x] framebuffer foundation:
    - [x] Tambah opt-in VGA mode 13h linear framebuffer (`320x200x8`).
    - [x] Implement bounded pixel, clear, rectangle, dan checksum primitives.
    - [x] Expose `/dev/fb0` geometry/write/clip/checksum diagnostics.
    - [x] Luluskan QEMU deterministic framebuffer render regression.
  - [x] compositor/server prototype:
    - [x] Surface registry dengan visibility dan z-order.
    - [x] Backbuffer composition dan bounded damage-region redraw.
    - [x] Cursor sebagai lapisan compositor teratas.
    - [x] Expose `/dev/compositor` frame/surface/damage/checksum diagnostics.
    - [x] Luluskan QEMU deterministic compositor regression.
- [ ] Advanced Graphics And Display Stack:
  - [x] Generic kernel display/KMS-style API:
    - [x] Model backend-neutral untuk display mode, stride, refresh rate, dan pixel format.
    - [x] Scanout-buffer allocation serta presentation/page-flip accounting.
    - [x] Pisahkan compositor daripada backend VGA melalui generic display API.
    - [x] Expose `/dev/display0` mode/buffer/flip/checksum diagnostics.
    - [x] Luluskan QEMU regression untuk display API, framebuffer, dan compositor.
  - [ ] UEFI GOP/VBE backend untuk framebuffer dinamik 24/32-bit.
  - [ ] EDID, output discovery, dan pemilihan mode paparan.
    - [x] Tambah parser EDID base block tervalidasi (header/checksum/identity/preferred timing).
    - [x] Model dan expose output firmware aktif serta status EDID melalui `/dev/display0`.
    - [ ] Bekalkan bytes EDID melalui DDC/backend GPU, enumerate multi-output, hotplug, dan modeset.
  - [ ] VirtIO-GPU PCI, control queue, resource, dan scanout backend.
    - [x] Discovery modern PCI transport, BAR, dan vendor capabilities serta expose
      `/dev/virtio-gpu` dengan regression QEMU `virtio-vga`.
    - [x] Feature negotiation dan control virtqueue foundation.
      - [x] Negotiate `VIRTIO_F_VERSION_1`, lengkapkan status handshake, dan enable
        queue 0 menggunakan descriptor/available/used DMA frames.
    - [ ] Display-info command, 2D resource, backing storage, transfer/flush, dan scanout.
  - [ ] Shared graphics buffers untuk proses user, fence, dan isolation.
  - [ ] Migrasi display server/compositor ke user mode.
  - [ ] Window protocol, input focus, font rendering, dan GUI toolkit.

### Phase E: Virtual Memory, SMP, And Scheduler Maturity

Matlamat phase ini ialah menjadikan kernel stabil di bawah beban sebenar dan bukan hanya
workload QEMU satu CPU.

- [x] Naikkan baseline kernel-thread stack daripada 8 KiB kepada 32 KiB selepas
  integration test menemui stack overflow yang merosakkan allocator semasa `init -> login`.
- [ ] Pindahkan kernel-thread stack kepada page-backed allocation dengan guard page,
  high-water diagnostics, dan kegagalan stack overflow yang deterministic.
- [ ] Gantikan fixed-size kernel heap dengan demand-grown page-backed heap yang boleh
  berkembang merentasi sempadan page table dengan selamat.
- [ ] Demand paging dan lazy allocation untuk executable, heap, dan stack user.
- [ ] `mmap`/`munmap`/`mprotect` serta memory-mapped file dengan W^X enforcement.
- [ ] Copy-on-write untuk `fork`, shared pages, dan page reference counting.
- [ ] Page cache bersepadu dengan VFS serta reclaim apabila memory pressure.
- [ ] Swap abstraction dan anonymous-page eviction sebagai feature opt-in.
- [ ] Out-of-memory policy yang boleh memilih dan menamatkan proses dengan selamat.
- [ ] ACPI discovery untuk MADT, HPET, MCFG, FADT, reboot, dan shutdown.
- [ ] APIC/x2APIC, IOAPIC, MSI, dan MSI-X; hentikan kebergantungan production pada PIC.
- [ ] SMP bootstrap untuk application processors dan per-CPU kernel state.
- [ ] SMP-safe scheduler dengan per-CPU run queue, affinity, migration, dan load balancing.
- [ ] Priority/niceness, starvation prevention, dan kelas real-time asas.
- [ ] Futex serta primitive blocking synchronization untuk pthread-compatible userland.
- [ ] High-resolution monotonic/realtime clocks, timer queue, dan tickless idle.
- [ ] CPU feature detection, XSAVE/FPU/SIMD context switching, dan topology reporting.
- [ ] Suspend, resume, CPU idle states, dan asas thermal/power management.

Exit criteria:

- [ ] Boot stabil dengan sekurang-kurangnya 4 vCPU dan jalankan stress scheduler/memory.
- [ ] Lulus ujian fork/COW, mmap, futex, OOM, dan concurrent filesystem/network I/O.
- [ ] Tiada global lock tunggal yang serialize semua process atau semua I/O hot path.

### Phase F: VFS And Storage Production Semantics

- [ ] Lengkapkan inode timestamp, UID/GID, mode bits, link count, dan access checks.
- [ ] Implement hard link, symbolic link, truncate, sparse file, dan atomic rename.
- [ ] Advisory file locking, directory iteration stabil, dan per-open file offsets.
- [ ] Mount/unmount lifecycle, bind mount, read-only mount, dan mount namespaces asas.
- [ ] Page cache, writeback, dirty-page throttling, `fsync`, dan block I/O scheduler.
- [ ] Crash-consistent VANTFS menggunakan journal atau copy-on-write metadata.
- [ ] Filesystem checker, repair utility, versioned on-disk format, dan backup metadata.
- [ ] GPT parser serta partition UUID/label discovery.
- [ ] FAT32 read/write untuk EFI/removable media interoperability.
- [ ] ISO9660 read-only untuk installer/live media.
- [ ] TRIM/discard dan SMART/health diagnostics untuk storage yang menyokongnya.
- [ ] Device hotplug-safe mount dan clean removal untuk USB/NVMe storage.

Exit criteria:

- [ ] Power-loss regression tidak merosakkan filesystem di luar transaksi aktif.
- [ ] Storage stress test merentas AHCI, NVMe, dan USB tanpa data mismatch.
- [ ] Permission dan ownership tests konsisten pada semua backend VFS.

### Phase G: POSIX-Like ABI And Native Userland Platform

- [ ] Tetapkan Vantara ABI yang versioned dan policy compatibility untuk syscall lama.
- [ ] Lengkapkan syscall process: `fork`, `execve`, `wait*`, process group, dan session.
- [ ] Lengkapkan descriptor API: `dup*`, `fcntl`, `ioctl`, `poll`, `select`, dan `epoll`-like.
- [ ] Lengkapkan filesystem API: `openat`, `statat`, cwd, symlink, mount, dan permissions.
- [ ] Lengkapkan time API, pipes, Unix-domain sockets, shared memory, dan futex.
- [ ] TTY/PTY subsystem dengan canonical/raw mode, terminal size, dan job control lengkap.
- [ ] Dynamic linker/loader, shared libraries, TLS, relocations, dan ASLR-compatible PIE.
- [ ] C ABI/toolchain target dan libc port atau libc compatibility layer.
- [ ] Rust target specification, standard-library port, dan versioned Vantara SDK.
- [ ] Native package format, dependency metadata, signed repository, dan package manager.
- [ ] Port shell dan core utilities yang cukup untuk build serta debug dari dalam Vantara.
- [ ] Sediakan stable headers, syscall documentation, examples, dan application test suite.

Exit criteria:

- [ ] Boleh compile dan menjalankan program C serta Rust bukan trivial di Vantara.
- [ ] Shell, pipes, redirection, PTY, background jobs, dan package install berfungsi.
- [ ] ABI regression menjamin binary lama terus berjalan dalam compatibility window.

### Phase H: Security, Identity, And Isolation

- [ ] User/group database, UID/GID supplementary groups, dan secure password hashing.
- [ ] Enforce permission, ownership, umask, sticky/setuid/setgid semantics secara end-to-end.
- [ ] Capability model untuk menggantikan kebergantungan mutlak kepada UID 0.
- [ ] Privilege separation untuk driver service, network service, login, dan compositor.
- [ ] Secure login/session lifecycle, credential switching, dan session auditing.
- [ ] Per-process resource limits serta quotas untuk memory, CPU, file, dan process count.
- [ ] Namespaces/sandbox asas untuk process, mount, IPC, network, dan device access.
- [ ] IPC access control dan explicit handle passing.
- [ ] ASLR kernel/user, stack canary, guard pages, NX, SMEP, SMAP, dan hardened allocator.
- [ ] Entropy collection dan cryptographically secure random generator melalui `/dev/random`.
- [ ] Verified/signed packages dan optional verified boot chain.
- [ ] Security event log, crash dump, vulnerability response, dan patch policy.
- [ ] Fuzz syscall, ELF, filesystem, USB, network parser, dan device emulation interfaces.

Exit criteria:

- [ ] Unprivileged process tidak boleh membaca memory, file, device, atau handle proses lain.
- [ ] Compromise satu service tidak memberi kawalan kernel atau keseluruhan desktop session.
- [ ] Security regression dan fuzz corpus menjadi release gate.

### Phase I: Complete Network Platform

- [ ] NIC framework dengan asynchronous RX/TX, interrupt moderation, scatter-gather, dan DMA safety.
- [ ] Driver VirtIO-net serta sekurang-kurangnya satu NIC hardware moden tambahan.
- [ ] DHCP client, DNS resolver, routing table, loopback, ICMP, dan raw diagnostics.
- [ ] IPv4 fragmentation/reassembly, Path MTU discovery, dan TCP congestion control matang.
- [ ] IPv6, neighbor discovery, SLAAC/DHCPv6, ICMPv6, dan dual-stack sockets.
- [ ] Socket options, non-blocking I/O, polling, multicast, Unix sockets, dan local IPC integration.
- [ ] Firewall/stateful packet filter, NAT, forwarding, dan per-interface policy.
- [ ] TLS library integration, certificate store, secure time bootstrap, dan HTTPS client.
- [ ] Wi-Fi stack: PCI/USB driver, 802.11 management, WPA2/WPA3 supplicant.
- [ ] Network manager user service dengan wired/wireless configuration dan diagnostics.
- [ ] Packet capture interface, counters, tracing, dan reproducible network stress tests.

Exit criteria:

- [ ] Vantara memperoleh alamat rangkaian dan mengakses HTTPS tanpa konfigurasi manual.
- [ ] TCP/UDP/IPv6 bertahan di bawah packet loss, reorder, reconnect, dan multi-process load.

### Phase J: Hardware Driver Coverage

- [ ] Driver model standard: probe/remove, dependency, power state, hotplug, dan stable device API.
- [ ] IOMMU abstraction dan DMA mapping API untuk isolation serta peranti melebihi DMA32.
- [ ] PCIe capability parsing, bridges, BAR allocation, MSI/MSI-X, dan hotplug.
- [ ] ACPI PCI routing, battery, lid, thermal zone, fan, dan power-button events.
- [ ] Input subsystem generik untuk keyboard, mouse, touchpad, touchscreen, dan game controller.
- [ ] Audio core serta Intel HDA/AC97 atau VirtIO-sound backend; mixer service userland.
- [ ] Bluetooth controller baseline dan HID/audio profile yang dipilih.
- [ ] RTC, hardware RNG, watchdog, GPIO/I2C/SPI abstraction mengikut target hardware.
- [ ] Printer/scanner/removable-device support melalui user-mode service apabila sesuai.
- [ ] Hardware compatibility database, driver binding rules, firmware loader, dan diagnostics.
- [ ] Ujian bare-metal pada sekurang-kurangnya satu desktop dan satu laptop rujukan.

Exit criteria:

- [ ] Boot, input, storage, network, display, audio, dan shutdown berfungsi pada mesin rujukan.
- [ ] Hotplug/unplug tidak panic kernel atau meninggalkan DMA/handle yang masih aktif.

### Phase K: Graphics, Desktop, And Human Interaction

- [ ] UEFI GOP/native linear framebuffer 24/32-bit dan mode setting berdasarkan EDID.
- [ ] VirtIO-GPU 2D sebagai backend virtualisasi utama.
- [ ] Kernel graphics memory manager, shared buffers, fences, page flip, dan process isolation.
- [ ] Pindahkan compositor/display server ke proses user yang unprivileged.
- [ ] Definisikan window/surface protocol dengan focus, resize, clipboard, drag-and-drop, dan IME.
- [ ] Input seat/session routing termasuk shortcut yang tidak boleh dipintas aplikasi biasa.
- [ ] Font rasterization, Unicode shaping, bidirectional text, DPI scaling, dan accessibility API.
- [ ] 2D rendering library, theme/widget toolkit, dan application lifecycle API.
- [ ] Desktop shell: panel, launcher, notification, settings, lock screen, dan file manager.
- [ ] Terminal emulator, text editor, image viewer, system monitor, dan network settings.
- [ ] GPU acceleration roadmap: VirtIO-GPU virgl/venus dahulu, kemudian satu keluarga GPU fizikal.
- [ ] Multi-monitor, hotplug display, vsync, damage tracking, dan software fallback.
- [ ] Audio/visual/input latency metrics serta desktop integration tests.

Exit criteria:

- [ ] Boot terus ke graphical login dan desktop tanpa menggunakan kernel shell.
- [ ] Beberapa aplikasi boleh berjalan serentak dengan input, clipboard, audio, dan network.
- [ ] Crash compositor atau aplikasi boleh dipulihkan tanpa reboot kernel.

### Phase L: Boot, Installer, Updates, And Recovery

- [ ] UEFI-native boot path dengan memory map, GOP, ACPI, initrd, dan kernel command line.
- [ ] GPT/EFI System Partition layout serta BIOS compatibility hanya jika diperlukan.
- [ ] Live/install image builder dengan partitioning, formatting, user creation, dan boot setup.
- [ ] Hardware discovery sebelum install dan laporan peranti/driver yang belum disokong.
- [ ] Atomic A/B atau snapshot-based system update dengan rollback.
- [ ] Signed update metadata, channel stable/beta/nightly, dan dependency resolution.
- [ ] Recovery environment, safe mode, filesystem repair, boot log, dan previous-kernel fallback.
- [ ] Persistent configuration migration dan rollback-safe package scripts.
- [ ] Reproducible ISO/disk images, SBOM, checksums, signatures, dan provenance metadata.

Exit criteria:

- [ ] Pengguna boleh install, boot, update, rollback, dan recover tanpa development tools.
- [ ] Interrupted installation/update tidak menghasilkan sistem yang tidak boleh boot.

### Phase M: Observability, Reliability, And Performance

- [ ] Structured kernel log dengan ring buffer, levels, timestamps, subsystem, dan rate limiting.
- [ ] Userspace logging daemon, rotation, persistent journal, dan diagnostics bundle.
- [ ] Kernel crash dump, symbolized stack traces, lock diagnostics, dan watchdog recovery.
- [ ] Tracing/profiling API untuk syscall, scheduler, allocation, I/O, network, dan IRQ latency.
- [ ] Sanitizer/debug builds, lock-order validator, race detector strategy, dan fault injection.
- [ ] Stress/soak tests untuk SMP, memory pressure, filesystem, network, USB hotplug, dan desktop.
- [ ] Performance benchmarks dan regression budgets untuk boot, context switch, I/O, dan rendering.
- [ ] CI matrix untuk QEMU BIOS/UEFI, multi-core, AHCI, NVMe, VirtIO, USB, dan failure scenarios.
- [ ] Bare-metal continuous test rack untuk hardware rujukan.
- [ ] Stable release criteria, long-term support policy, changelog, dan incident process.

Exit criteria:

- [ ] 24-hour mixed-workload soak test tanpa panic, leak kritikal, atau filesystem corruption.
- [ ] Setiap kernel panic menghasilkan diagnostic artifact yang boleh dianalisis.
- [ ] Release tidak dibuat jika correctness, security, compatibility, atau performance gate gagal.

### Phase N: Ecosystem And Self-Hosting

- [ ] Source control client, build tools, compiler/linker, debugger, dan package tooling native.
- [ ] Port library asas: compression, crypto, TLS, Unicode, image, audio, dan database ringan.
- [ ] Application permission/manifest model dan stable desktop integration APIs.
- [ ] Developer documentation, API reference, samples, templates, dan emulator workflow.
- [ ] Package repository automation, review, signing, reproducibility, dan vulnerability scanning.
- [ ] Build sebahagian userland Vantara dari dalam Vantara sendiri.
- [ ] Capai staged self-hosting untuk SDK dan akhirnya keseluruhan base system.

Exit criteria:

- [ ] Developer boleh membina, menguji, debug, package, dan memasang aplikasi dalam Vantara.
- [ ] Base system boleh dibina semula secara reproducible menggunakan toolchain Vantara.

## Definition Of "Linux-Like Usable"

Vantara tidak perlu menyalin Linux atau menyokong semua hardwarenya. Sasaran minimum
general-purpose OS dianggap tercapai apabila:

- [ ] Boot dan install pada UEFI VM serta sekurang-kurangnya dua mesin x86_64 rujukan.
- [ ] SMP, process isolation, permissions, networking, storage, audio, input, dan graphics stabil.
- [ ] Graphical login, desktop, terminal, file manager, settings, dan aplikasi asas tersedia.
- [ ] Sambungan Ethernet atau Wi-Fi, DNS, DHCP, TLS, dan web/API client boleh digunakan.
- [ ] Sistem boleh update serta rollback dan mempunyai recovery path tanpa rebuild manual.
- [ ] SDK C/Rust, libc/POSIX subset, package manager, dan dokumentasi aplikasi stabil.
- [ ] Security, fuzzing, stress, performance, dan compatibility suites menjadi release gate.

## Recommended Execution Order

Urutan mengurangkan kerja ulang dan membawa UI ke skrin tanpa mengorbankan foundation:

1. [ ] Siapkan UEFI GOP 32-bit, EDID, dan VirtIO-GPU dalam Advanced Graphics.
2. [ ] Implement shared graphics buffers/fences dan pindahkan compositor ke user mode.
3. [ ] Jalankan Phase E secara selari mengikut dependency: ACPI/APIC/SMP, VM, kemudian futex.
4. [ ] Lengkapkan permission-aware VFS dan crash consistency dalam Phase F.
5. [ ] Stabilkan POSIX-like ABI, libc, dynamic linker, TTY/PTY, dan SDK dalam Phase G.
6. [ ] Bina security boundary Phase H sebelum membuka package/app ecosystem.
7. [ ] Lengkapkan network dan driver mesin rujukan dalam Phase I/J.
8. [ ] Siapkan desktop Phase K, kemudian installer/update/recovery Phase L.
9. [ ] Jadikan reliability gates Phase M wajib sebelum stable release.
10. [ ] Kejar ecosystem dan self-hosting Phase N selepas ABI stabil.

## Suggested Next Sprint

Sprint seterusnya fokus menghasilkan paparan moden yang menjadi asas desktop:

- [x] Tambah bootloader framebuffer handoff dan backend UEFI GOP 32-bit.
- [x] Generalize compositor supaya tidak bergantung pada konstanta VGA `320x200`.
- [x] Implement mode/stride/pixel-format validation dan bounded 24/32-bit pixel operations.
- [x] Tambah QEMU OVMF regression untuk render, checksum, clipping, dan page flip.
- [x] Expose geometry/backend sebenar melalui `/dev/fb0` dan `/dev/display0`.
  - [x] PID 1 baca kedua-dua node melalui syscall `open/read/close` dan regression
    UEFI sahkan geometry `1280x800x32`, stride, backend, checksum, buffer, serta flip.
  - [ ] Ujian shell interaktif UEFI selepas migrasi interrupt input daripada PIC kepada
    APIC/IOAPIC; Q35 belum menghantar keyboard IRQ dengan laluan PIC legacy semasa.
- [x] Dokumentasikan display ownership, shared-buffer threat model, dan laluan ke user compositor
  dalam `docs/display-architecture.md`.
