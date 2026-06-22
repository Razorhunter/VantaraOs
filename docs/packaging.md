# Packaging And Initrd

Vantara OS produces a deterministic development package with:

```text
vantara-dev/
├── README.txt
├── boot/
│   ├── vantara-kernel.img
│   └── vantara-initrd.tar
├── disk/
│   └── vantara-persist.img
└── metadata/
    ├── artifact-manifest.tsv
    ├── build-metadata.tsv
    ├── initrd-manifest.tsv
    └── package-manifest.tsv
```

Build and verify it with:

```bash
make package-test
```

The shareable archive is written to:

```text
target/vantara-dev.tar.gz
```

Set `SOURCE_DATE_EPOCH` to produce stable archive timestamps. The initrd and
package use sorted paths, numeric root ownership, fixed timestamps, and gzip
without embedded wall-clock metadata.

The initrd contains one canonical `/bin` image per program, selected from the
same generated registry used by the kernel. This prevents stale `.bin` and
`.elf` variants from entering release artifacts.

The current bootloader-compatible image still embeds userland in the kernel.
`vantara-initrd.tar` establishes the package and integrity contract for a future
boot-module handoff; it is not yet consumed during boot.
