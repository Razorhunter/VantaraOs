# Vantara OS Dev Image Release Checklist

Use this checklist before sharing or archiving a Vantara OS development image.
Run release commands from the repository root, preferably inside the Docker
builder at `/workspace`.

## 1. Choose Release Metadata

- [ ] Confirm the version in `kernel/Cargo.toml`.
- [ ] Record the source commit or explicitly set `VANTARA_GIT_COMMIT`.
- [ ] Confirm whether the source tree is clean, or explicitly set
      `VANTARA_GIT_DIRTY`.
- [ ] Choose a reproducible build timestamp using `SOURCE_DATE_EPOCH`.

Example:

```bash
export SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)"
```

If the source is not in a Git repository, provide the metadata explicitly:

```bash
export VANTARA_GIT_COMMIT="development"
export VANTARA_GIT_DIRTY="true"
```

## 2. Build And Static Checks

- [ ] Run formatting and compile checks:

```bash
make fmt
make check
```

- [ ] Build the boot image and refresh the artifact manifest:

```bash
make kernel-build
```

- [ ] Confirm these files exist:

```text
kernel/target/x86_64-vantara_os/debug/bootimage-kernel.bin
target/generated/build-metadata.tsv
target/generated/userland_images.rs
target/artifact-manifest.tsv
```

## 3. QEMU Regression Suite

- [ ] Core boot smoke test passes:

```bash
make smoke
```

- [ ] `/bin/init -> login -> sh` integration test passes:

```bash
make boot-test
```

- [ ] Golden-output command tests pass for `ls`, `cat`, `procs`, and
      `rusthello`:

```bash
make command-smoke
```

- [ ] Null and stack-guard page-fault isolation tests pass:

```bash
make isolation-test
```

The complete local sequence is also available as:

```bash
make regression
```

`make ci` runs the same regression gate.

The current nightly custom-target toolchain compile-checks the in-kernel
`#[test_case]` harness. Runtime regression authority comes from the QEMU boot,
command, process, memory-isolation, syscall-pointer, and CPU-exception suites.

## 4. Inspect Release Artifacts

- [ ] Review `target/generated/build-metadata.tsv`.
- [ ] Confirm version, Git state, and timestamp are expected.
- [ ] Review `target/artifact-manifest.tsv`.
- [ ] Confirm the manifest includes one kernel image, build metadata, generated
      registry, and every intended userland `.bin`/`.elf`.
- [ ] Verify every manifest size and SHA-256 digest:

```bash
set -euo pipefail
tail -n +2 target/artifact-manifest.tsv |
while IFS=$'\t' read -r type path bytes digest; do
  test -f "$path"
  test "$(stat -c '%s' "$path")" = "$bytes"
  actual="$(sha256sum "$path")"
  test "${actual%% *}" = "$digest"
done
```

## 5. Manual Boot Sanity Check

- [ ] Boot the image interactively:

```bash
make kernel-run
```

- [ ] Confirm the serial/VGA banner shows the expected build metadata.
- [ ] Log in as `root`.
- [ ] Confirm the shell prompt, keyboard input, and representative commands
      behave normally.
- [ ] Confirm there is no `KERNEL PANIC` in the release logs.

## 6. Package And Handoff

- [ ] Keep the boot image and `target/artifact-manifest.tsv` together.
- [ ] Include `target/generated/build-metadata.tsv`.
- [ ] Include test logs when the image is intended for debugging or review.
- [ ] State clearly that this is a development image, not a production or
      security-hardened operating system.
- [ ] Record known limitations and any skipped test before distribution.

The dev image is release-ready only after every applicable item above passes.
