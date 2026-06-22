# QEMU

Common Vantara OS QEMU commands.

## Run

```bash
make run
```

Connect to:

```text
localhost:5900
```

The run target attaches:

- the kernel boot image as IDE disk 0;
- `target/vantara-persist.img` as IDE disk 1.

The persistence image is created only when absent. Re-running QEMU preserves
files written under `/persist`.

Reset the volume explicitly:

```bash
make reset-persist
```

Run QEMU from the Docker builder:

```bash
make docker-run
```

Then connect a VNC client to `localhost:5901`. Override the Docker port/display
when needed:

```bash
make docker-run DOCKER_VNC_PORT=5902 DOCKER_VNC_DISPLAY=2
```

## Smoke Test

```bash
make smoke
make boot-test
make command-smoke
make filesystem-write-test
make filesystem-persistence-test
make isolation-test
make artifact-manifest
```

The smoke test builds the bootimage if needed, runs QEMU headless, captures
serial output in `target/qemu-smoke.log`, and checks for core boot markers.
The boot integration test logs in as `root` and verifies the init-to-shell path.
The command smoke tests use a fresh guest and a separate serial log for each
command. Individual cases can be run with `make command-smoke-ls`,
`make command-smoke-cat`, `make command-smoke-procs`, or
`make command-smoke-rusthello`.
The isolation test triggers null and stack-guard page faults from Ring 3 and
checks that the shell can still launch another process afterward.
Every kernel build also writes `target/artifact-manifest.tsv`. The manifest
contains deterministic, path-sorted SHA-256 records for the boot image,
generated build metadata, userland registry, and userland binaries. Build
metadata is stored in `target/generated/build-metadata.tsv` and displayed in
the serial boot banner. Use `SOURCE_DATE_EPOCH` to attach a reproducible UTC
build timestamp.

If QEMU is installed under a different name or path:

```bash
make QEMU=/path/to/qemu-system-x86_64 smoke
```
