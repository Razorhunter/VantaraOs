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
- [ ] USB roadmap split:
  - [x] detect UHCI via PCI instead of hardcoded I/O base
  - [x] initialize controller only if found
  - [x] enumerate root ports
  - [ ] parse descriptors
  - [ ] support HID keyboard/mouse later

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

- [x] Tambah `/bin/init` minimal sebagai userland process.
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
- [x] Tambah init reaper untuk collect orphan/kernel-parented zombie process.
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

### Phase B: Real Multitasking Platform

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

### Phase C: Persistent System Foundation

- [ ] Writable filesystem dengan metadata dan directory tree sebenar.
- [ ] VFS layer supaya filesystem backend boleh ditukar.
- [ ] Init/service manager yang boleh spawn dan monitor services.
- [ ] Device namespace:
  - [ ] `/dev`
  - [ ] driver registry
  - [ ] user-visible device info
- [ ] Package/build layout untuk kernel + userland + initrd.

### Phase D: Hardware And I/O Maturity

- [ ] Storage driver path:
  - [ ] ATA/AHCI/NVMe roadmap
  - [ ] block cache
  - [ ] partition parser
- [ ] Network stack roadmap:
  - [ ] NIC driver
  - [ ] Ethernet
  - [ ] ARP
  - [ ] IPv4
  - [ ] UDP/TCP
- [ ] USB HID dan mass storage.
- [ ] Graphics mode selepas VGA text:
  - [ ] framebuffer
  - [ ] compositor/server prototype

### Phase E: Security, Users, And Compatibility

- [ ] User/group permission model.
- [ ] File permissions.
- [ ] Capability atau privilege boundary untuk sensitive syscall.
- [ ] Secure login/session model.
- [ ] Compatibility layer target:
  - [ ] POSIX-like subset
  - [ ] Vantara-native ABI
  - [ ] Rust userland SDK

### Phase F: Product-Level OS Experience

- [ ] Installer/dev image builder.
- [ ] Shell utilities suite.
- [ ] Service logs dan diagnostics command.
- [ ] Documentation untuk app developer.
- [ ] Release cadence:
  - [ ] nightly/dev image
  - [ ] milestone image
  - [ ] changelog

## Suggested Next Sprint

Sprint paling berbaloi selepas state sekarang:

- [ ] Mulakan Milestone 18 dengan centralized user pointer validation.
- [ ] Tambah `/bin/fault` scenario untuk invalid syscall pointer.
- [ ] Tambah tests pointer validation sebelum scheduler preemption.
