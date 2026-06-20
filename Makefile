KERNEL_DIR := kernel
USERLAND_NATIVE_DIR := userland/native
USERLAND_BIN_DIR := target/userland
TARGET     := x86_64-vantara_os
BOOTIMAGE  := $(KERNEL_DIR)/target/$(TARGET)/debug/bootimage-kernel.bin
QEMU       := qemu-system-x86_64

.PHONY: help build shell docker-preemption-test docker-regression fmt check test run smoke boot-test command-smoke command-smoke-ls command-smoke-cat command-smoke-procs command-smoke-rusthello command-smoke-threaddemo command-smoke-pipedemo command-smoke-eventdemo command-smoke-msgdemo command-smoke-jobdemo preemption-test kernel-thread-preemption-test signal-test terminal-signal-test isolation-test artifact-manifest abi-check unsafe-audit regression ci kernel-fmt kernel-check kernel-test kernel-build kernel-run kernel-vnc kernel-headless userland-fmt userland-check userland-bin kernel-clean clean

help:
	@echo "Vantara OS Build System"
	@echo ""
	@echo "Usage:"
	@echo "make build            - Build Docker image for Vantara OS building"
	@echo "make shell            - Enter Docker builder"
	@echo "make docker-preemption-test - Run preemption test fully inside Docker"
	@echo "make docker-regression - Run the full regression suite inside Docker"
	@echo "make clean            - Clean all builds"
	@echo "make fmt              - Format kernel Rust code"
	@echo "make check            - Check kernel and integration tests"
	@echo "make test             - Compile-check kernel test targets"
	@echo "make smoke            - Run headless QEMU boot smoke test"
	@echo "make boot-test        - Test /bin/init -> login -> sh in QEMU"
	@echo "make command-smoke     - Test golden output for key userland commands"
	@echo "make preemption-test   - Test timer preemption of a CPU-bound user process"
	@echo "make kernel-thread-preemption-test - Test timer switching between Ring-0 threads"
	@echo "make signal-test       - Test SIGTERM delivery to a background job"
	@echo "make terminal-signal-test - Test Ctrl-C/Ctrl-Z foreground job signals"
	@echo "make isolation-test    - Test user page-fault containment in QEMU"
	@echo "make artifact-manifest - Build and checksum release artifacts"
	@echo "make abi-check         - Verify kernel/userland syscall ABI constants"
	@echo "make unsafe-audit      - Verify reviewed unsafe Rust baseline"
	@echo "make regression        - Run full process/memory/syscall/fs/boot suite"
	@echo "make ci               - Run local CI checks"
	@echo "make run              - Run kernel"
	@echo "make kernel-fmt       - Format kernel Rust code"
	@echo "make kernel-check     - Check kernel and integration tests"
	@echo "make kernel-test      - Compile-check kernel test targets"
	@echo "make kernel-build     - Build kernel"
	@echo "make kernel-run       - Run kernel"
	@echo "make kernel-headless  - Run kernel serial only"
	@echo "make userland-fmt     - Format native userland sources"
	@echo "make userland-check   - Check native userland sources"
	@echo "make userland-bin     - Build native /bin command images"

build:
	docker compose build vantara-kernel-builder --no-cache

shell:
	docker compose run --rm --service-ports vantara-kernel-builder

docker-preemption-test:
	docker compose run --rm vantara-kernel-builder make preemption-test

docker-regression:
	docker compose run --rm vantara-kernel-builder make regression

fmt: kernel-fmt

check: abi-check unsafe-audit kernel-check

test: kernel-test

run: kernel-run

smoke: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-smoke.sh

boot-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-boot-integration.sh

command-smoke: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-command-smoke.sh

command-smoke-ls command-smoke-cat command-smoke-procs command-smoke-rusthello command-smoke-threaddemo command-smoke-pipedemo command-smoke-eventdemo command-smoke-msgdemo command-smoke-jobdemo: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-command-smoke.sh $(@:command-smoke-%=%)

preemption-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-preemption.sh

kernel-thread-preemption-test: userland-bin
	QEMU="$(QEMU)" bash scripts/qemu-kernel-thread-preemption.sh

signal-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-signal-job-control.sh

terminal-signal-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-terminal-signals.sh

isolation-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-fault-isolation.sh

ci:
	bash scripts/ci.sh

regression:
	QEMU="$(QEMU)" bash scripts/regression.sh

kernel-fmt:
	cd $(KERNEL_DIR) && cargo fmt

kernel-check: userland-bin
	cd $(KERNEL_DIR) && cargo check --tests

kernel-test: userland-bin
	cd $(KERNEL_DIR) && cargo check --tests

kernel-build: userland-bin
	cd $(KERNEL_DIR) && cargo bootimage
	bash scripts/generate-artifact-manifest.sh

artifact-manifest: kernel-build

abi-check:
	bash scripts/check-syscall-abi.sh

unsafe-audit:
	bash scripts/check-unsafe-audit.sh

kernel-run: kernel-build
	$(QEMU) \
		-drive format=raw,file=$(BOOTIMAGE) \
		-vnc 0.0.0.0:0 \
		-serial stdio

kernel-headless: kernel-build
	$(QEMU) \
		-drive format=raw,file=$(BOOTIMAGE) \
		-display none \
		-serial stdio

userland-fmt:
	cd $(USERLAND_NATIVE_DIR) && cargo fmt

userland-check:
	cd $(USERLAND_NATIVE_DIR) && cargo check

$(USERLAND_BIN_DIR):
	mkdir -p $(USERLAND_BIN_DIR)

$(USERLAND_BIN_DIR)/%.bin: $(USERLAND_NATIVE_DIR)/asm/%.asm $(USERLAND_NATIVE_DIR)/asm/abi.inc | $(USERLAND_BIN_DIR)
	nasm -f bin -I $(USERLAND_NATIVE_DIR)/asm/ $< -o $@

$(USERLAND_BIN_DIR)/%.elf: $(USERLAND_NATIVE_DIR)/elf/%.rs $(USERLAND_NATIVE_DIR)/elf/linker.ld $(USERLAND_NATIVE_DIR)/src/abi.rs | $(USERLAND_BIN_DIR)
	rustc --edition=2024 --target x86_64-unknown-none -C panic=abort -C no-redzone=yes -C relocation-model=static -C link-arg=-T$(USERLAND_NATIVE_DIR)/elf/linker.ld -C link-arg=-nostdlib $< -o $@

userland-bin: $(USERLAND_BIN_DIR)/demo.bin $(USERLAND_BIN_DIR)/uptime.bin $(USERLAND_BIN_DIR)/ls.bin $(USERLAND_BIN_DIR)/cat.bin $(USERLAND_BIN_DIR)/whoami.bin $(USERLAND_BIN_DIR)/fault.elf $(USERLAND_BIN_DIR)/init.bin $(USERLAND_BIN_DIR)/login.elf $(USERLAND_BIN_DIR)/sh.elf $(USERLAND_BIN_DIR)/stat.elf $(USERLAND_BIN_DIR)/procs.elf $(USERLAND_BIN_DIR)/pci.elf $(USERLAND_BIN_DIR)/netdev.elf $(USERLAND_BIN_DIR)/dmesg.elf $(USERLAND_BIN_DIR)/drvstat.elf $(USERLAND_BIN_DIR)/kill.elf $(USERLAND_BIN_DIR)/sleep.elf $(USERLAND_BIN_DIR)/yielddemo.elf $(USERLAND_BIN_DIR)/preemptdemo.elf $(USERLAND_BIN_DIR)/threaddemo.elf $(USERLAND_BIN_DIR)/pipedemo.elf $(USERLAND_BIN_DIR)/eventdemo.elf $(USERLAND_BIN_DIR)/msgdemo.elf $(USERLAND_BIN_DIR)/signaldemo.elf $(USERLAND_BIN_DIR)/jobdemo.elf $(USERLAND_BIN_DIR)/rusthello.elf $(USERLAND_BIN_DIR)/pwd.elf

kernel-clean:
	cd $(KERNEL_DIR) && cargo clean

clean:
	docker compose run --rm vantara-kernel-builder \
		bash -c "cd $(KERNEL_DIR) && cargo clean"
