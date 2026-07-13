KERNEL_DIR := kernel
KERNEL_TARGET_DIR := target/kernel
KERNEL_TARGET_SPEC := target/generated/x86_64-vantara_os.json
USERLAND_DIR := userland/native
USERLAND_BIN_DIR := target/userland
TARGET := x86_64-vantara_os
CARGO_PATH := /usr/local/cargo/bin:$(PATH)
BOOTIMAGE := $(KERNEL_TARGET_DIR)/$(TARGET)/debug/bootimage-kernel.bin

QEMU ?= qemu-system-x86_64
PERSIST_IMAGE ?= target/vantara-persist.img
PERSIST_SIZE ?= 1M
INITRD ?= target/vantara-initrd.tar
DEV_PACKAGE ?= target/vantara-dev.tar.gz
VNC_DISPLAY ?= 0
DOCKER_VNC_DISPLAY ?= 1
DOCKER_VNC_PORT ?= 5901
DOCKER_VNC_GUEST_PORT = $(shell expr 5900 + $(DOCKER_VNC_DISPLAY))

QEMU_DRIVES := \
	-drive format=raw,file=$(BOOTIMAGE),index=0,media=disk \
	-drive format=raw,file=$(PERSIST_IMAGE),index=1,media=disk

ASM_PROGRAMS := demo uptime ls cat whoami touch write rm mv mkdir rmdir
ELF_PROGRAMS := \
	init fault login sh stat procs pci netdev dmesg drvstat kill sleep \
	yielddemo preemptdemo threaddemo pipedemo eventdemo msgdemo \
	signaldemo jobdemo udpdemo tcpdemo rusthello pwd
USERLAND_IMAGES := \
	$(addprefix $(USERLAND_BIN_DIR)/,$(addsuffix .bin,$(ASM_PROGRAMS))) \
	$(addprefix $(USERLAND_BIN_DIR)/,$(addsuffix .elf,$(ELF_PROGRAMS)))

COMMAND_SMOKE_CASES := \
	ls cat procs rusthello threaddemo pipedemo eventdemo msgdemo jobdemo udpdemo tcpdemo
COMMAND_SMOKE_TARGETS := $(addprefix command-smoke-,$(COMMAND_SMOKE_CASES))

.DEFAULT_GOAL := help

.PHONY: \
	help \
	build docker-build shell docker-run docker-regression docker-preemption-test \
	fmt check test kernel-fmt kernel-check kernel-test userland-fmt userland-check \
	kernel-target-spec kernel-build userland-bin artifact-manifest initrd package package-test \
	abi-check unsafe-audit \
	run \
	persist-disk reset-persist \
	smoke boot-test command-smoke $(COMMAND_SMOKE_TARGETS) \
	service-manager-test device-namespace-test network-rx-test \
	filesystem-write-test filesystem-persistence-test block-cache-test partition-test \
	ahci-test ahci-write-test ahci-vantfs-test ahci-persist-test storage-policy-test nvme-test nvme-write-test preemption-test \
	kernel-thread-preemption-test signal-test terminal-signal-test isolation-test \
	regression ci \
	clean kernel-clean userland-clean

help:
	@echo "Vantara OS"
	@echo ""
	@echo "Build:"
	@echo "  make kernel-build        Build userland, kernel boot image, and manifest"
	@echo "  make initrd              Build canonical userland initrd artifact"
	@echo "  make package             Build shareable development package"
	@echo "  make package-test        Verify packaged files and checksums"
	@echo "  make check               Run ABI, unsafe, and compile checks"
	@echo "  make fmt                 Format kernel and userland Rust sources"
	@echo ""
	@echo "Run (persistent /persist disk attached):"
	@echo "  make run                 QEMU VNC server on localhost:5900"
	@echo "  make reset-persist       Recreate the persistent disk (destroys its data)"
	@echo ""
	@echo "Tests:"
	@echo "  make smoke"
	@echo "  make boot-test"
	@echo "  make service-manager-test"
	@echo "  make device-namespace-test"
	@echo "  make network-rx-test"
	@echo "  make command-smoke"
	@echo "  make filesystem-write-test"
	@echo "  make filesystem-persistence-test"
	@echo "  make block-cache-test"
	@echo "  make partition-test"
	@echo "  make ahci-test"
	@echo "  make ahci-write-test"
	@echo "  make ahci-vantfs-test"
	@echo "  make ahci-persist-test"
	@echo "  make storage-policy-test"
	@echo "  make nvme-test"
	@echo "  make nvme-write-test"
	@echo "  make preemption-test"
	@echo "  make kernel-thread-preemption-test"
	@echo "  make signal-test"
	@echo "  make terminal-signal-test"
	@echo "  make isolation-test"
	@echo "  make regression"
	@echo ""
	@echo "Docker:"
	@echo "  make docker-build"
	@echo "  make shell"
	@echo "  make docker-run          VNC on localhost:5901 (override DOCKER_VNC_PORT)"
	@echo "  make docker-regression"

# Docker workflow

build: docker-build

docker-build:
	docker compose build vantara-kernel-builder

shell:
	docker compose run --rm --service-ports vantara-kernel-builder

docker-run:
	docker compose run --rm \
		-p $(DOCKER_VNC_PORT):$(DOCKER_VNC_GUEST_PORT) \
		vantara-kernel-builder \
		make run VNC_DISPLAY=$(DOCKER_VNC_DISPLAY)

docker-regression:
	docker compose run --rm vantara-kernel-builder make regression

docker-preemption-test:
	docker compose run --rm vantara-kernel-builder make preemption-test

# Formatting and checks

fmt: kernel-fmt userland-fmt

check: abi-check unsafe-audit kernel-check userland-check

test: kernel-test

kernel-fmt:
	cd $(KERNEL_DIR) && PATH="$(CARGO_PATH)" cargo fmt

userland-fmt:
	cd $(USERLAND_DIR) && cargo fmt

kernel-check: userland-bin kernel-target-spec
	cd $(KERNEL_DIR) && PATH="$(CARGO_PATH)" CARGO_TARGET_DIR=../$(KERNEL_TARGET_DIR) cargo check --tests --target ../$(KERNEL_TARGET_SPEC)

kernel-test: userland-bin kernel-target-spec
	cd $(KERNEL_DIR) && PATH="$(CARGO_PATH)" CARGO_TARGET_DIR=../$(KERNEL_TARGET_DIR) cargo check --tests --target ../$(KERNEL_TARGET_SPEC)

userland-check:
	cd $(USERLAND_DIR) && PATH="$(CARGO_PATH)" cargo check

abi-check:
	bash scripts/check-syscall-abi.sh

unsafe-audit:
	bash scripts/check-unsafe-audit.sh

# Build

kernel-target-spec:
	bash scripts/generate-rust-target.sh "$(KERNEL_TARGET_SPEC)"
	cd $(KERNEL_DIR) && PATH="$(CARGO_PATH)" cargo fetch --target ../$(KERNEL_TARGET_SPEC)
	bash scripts/patch-bootloader-target.sh "$(KERNEL_TARGET_SPEC)"

kernel-build: userland-bin kernel-target-spec
	cd $(KERNEL_DIR) && PATH="$(CARGO_PATH)" CARGO_TARGET_DIR=../$(KERNEL_TARGET_DIR) cargo bootimage --target ../$(KERNEL_TARGET_SPEC)
	bash scripts/generate-artifact-manifest.sh

artifact-manifest: kernel-build

initrd: kernel-build
	bash scripts/build-initrd.sh

package: initrd
	PERSIST_SIZE="$(PERSIST_SIZE)" bash scripts/build-dev-package.sh

package-test: package
	bash scripts/verify-dev-package.sh

$(USERLAND_BIN_DIR):
	mkdir -p $@

$(USERLAND_BIN_DIR)/%.bin: $(USERLAND_DIR)/asm/%.asm $(USERLAND_DIR)/asm/abi.inc | $(USERLAND_BIN_DIR)
	nasm -f bin -I $(USERLAND_DIR)/asm/ $< -o $@

$(USERLAND_BIN_DIR)/%.elf: $(USERLAND_DIR)/elf/%.rs $(USERLAND_DIR)/elf/linker.ld $(USERLAND_DIR)/src/abi.rs | $(USERLAND_BIN_DIR)
	rustc --edition=2024 --target x86_64-unknown-none \
		-C panic=abort \
		-C no-redzone=yes \
		-C relocation-model=static \
		-C link-arg=-T$(USERLAND_DIR)/elf/linker.ld \
		-C link-arg=-nostdlib \
		$< -o $@

userland-bin: $(USERLAND_IMAGES)
	rm -f \
		$(USERLAND_BIN_DIR)/init.bin \
		$(USERLAND_BIN_DIR)/touch.elf \
		$(USERLAND_BIN_DIR)/write.elf \
		$(USERLAND_BIN_DIR)/rm.elf \
		$(USERLAND_BIN_DIR)/mv.elf

# Persistent disk and QEMU

persist-disk:
	@mkdir -p $(dir $(PERSIST_IMAGE))
	@if [ ! -f "$(PERSIST_IMAGE)" ]; then \
		truncate -s "$(PERSIST_SIZE)" "$(PERSIST_IMAGE)"; \
		echo "created persistent disk: $(PERSIST_IMAGE) ($(PERSIST_SIZE))"; \
	else \
		echo "using persistent disk: $(PERSIST_IMAGE)"; \
	fi

reset-persist:
	rm -f "$(PERSIST_IMAGE)"
	$(MAKE) persist-disk

run: kernel-build persist-disk
	$(QEMU) \
		$(QEMU_DRIVES) \
		-display none \
		-vnc 0.0.0.0:$(VNC_DISPLAY) \
		-serial stdio

# QEMU tests

smoke: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-smoke.sh

boot-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-boot-integration.sh

service-manager-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-service-manager.sh

device-namespace-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-device-namespace.sh

network-rx-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-network-rx.sh

command-smoke: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-command-smoke.sh

$(COMMAND_SMOKE_TARGETS): kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-command-smoke.sh $(@:command-smoke-%=%)

filesystem-write-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-filesystem-write.sh

filesystem-persistence-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-filesystem-persistence.sh

block-cache-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-block-cache.sh

partition-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-partition-parser.sh

ahci-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-ahci-discovery.sh

ahci-write-test: userland-bin
	QEMU="$(QEMU)" bash scripts/qemu-ahci-write-persistence.sh

ahci-vantfs-test: userland-bin
	QEMU="$(QEMU)" bash scripts/qemu-ahci-vantfs-persistence.sh

ahci-persist-test: userland-bin
	QEMU="$(QEMU)" bash scripts/qemu-ahci-persist-lifecycle.sh

storage-policy-test: userland-bin
	QEMU="$(QEMU)" bash scripts/qemu-storage-auto-policy.sh

nvme-test: kernel-build
	QEMU="$(QEMU)" bash scripts/qemu-nvme-discovery.sh

nvme-write-test: userland-bin
	QEMU="$(QEMU)" bash scripts/qemu-nvme-write-persistence.sh

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

regression:
	QEMU="$(QEMU)" bash scripts/regression.sh

ci:
	bash scripts/ci.sh

# Cleanup

kernel-clean:
	cd $(KERNEL_DIR) && PATH="$(CARGO_PATH)" CARGO_TARGET_DIR=../$(KERNEL_TARGET_DIR) cargo clean

userland-clean:
	rm -rf $(USERLAND_BIN_DIR)

clean: kernel-clean userland-clean
	rm -rf \
		target/generated \
		target/artifact-manifest.tsv \
		target/initrd-manifest.tsv \
		$(INITRD) \
		$(DEV_PACKAGE)
