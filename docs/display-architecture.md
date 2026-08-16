# VantaraOS Display Architecture

## Current ownership

The bootloader or firmware selects the initial video mode and keeps the linear
framebuffer mapped for the lifetime of the kernel. The framebuffer driver owns
the validated address, geometry, stride, pixel format, bounded writes, and the
final volatile copy into scanout memory.

The display layer owns backend-neutral mode metadata, allocation accounting,
presentation validation, page-flip accounting, and `/dev/display0`. It does not
give callers a raw pointer to firmware framebuffer memory.

The kernel compositor currently owns one heap-backed scanout buffer and the
surface registry. It draws only through bounded geometry and submits complete
buffers through the display layer. `/dev/fb0`, `/dev/display0`, and
`/dev/compositor` are read-only diagnostic snapshots; they are not memory map
or rendering APIs.

## Invariants

- A backend registration is rejected for a null address, zero geometry,
  unsupported bits per pixel, an undersized stride, or size overflow.
- `stride` is always bytes per row inside the kernel display API.
- A presented buffer must be exactly `stride * height` bytes.
- The framebuffer address never crosses the user/kernel boundary as a usable
  handle. Its diagnostic hexadecimal value is informational only.
- Only the display backend writes hardware scanout memory. The compositor and
  future user display server submit owned buffers.
- Mode changes must invalidate old buffers and fences before accepting another
  presentation.

## Shared-buffer threat model

Future clients and the user-mode compositor are mutually untrusted. A client
may supply malicious dimensions, stride, damage rectangles, offsets, formats,
fence values, or continue using a buffer after destruction. A compromised
compositor must not gain arbitrary physical-memory access or map buffers owned
by another session.

The shared-buffer ABI must therefore use opaque kernel handles rather than
physical addresses. Creation validates overflow-safe size and per-process
memory limits. Mapping records owner process/session and read/write rights.
Every submit validates handle ownership, current generation, format, stride,
dimensions, damage bounds, and fence state. Destruction revokes new operations;
actual storage is reclaimed only after outstanding references and device work
complete. Buffers are cleared before first use and before transfer to a
different security domain.

The kernel display backend remains the final authority for modeset and scanout.
It must reject stale, cross-process, oversized, executable, or device-incompatible
buffers and expose counters for rejected submissions.

## Migration to a user compositor

1. Add output discovery and EDID parsing while the kernel compositor remains
   the only presenter.
2. Add kernel-managed graphics-buffer handles, mapping permissions, reference
   counts, fences, and per-process quotas.
3. Define a versioned display protocol for surface creation, damage, input
   focus, frame callbacks, and output changes.
4. Start a privileged user display server after devfs and input services. Give
   it the modeset/scanout capability, not unrestricted physical memory.
5. Move composition and cursor policy to that server. Keep a minimal kernel
   panic/boot renderer independent of the user display service.
6. Add restart recovery: revoke the failed server's handles, blank or retain a
   known-good scanout, then authorize a replacement server.

## Current limitations

ABI v1.16 provides the first PID-owned surface lifecycle: create,
configure, palette-color update, focus, and destroy. Cross-process mutation is
rejected, surface geometry and count are bounded, and `/bin/windowdemo` covers
the Ring-3 to VirtIO-GPU path. Owned surfaces are reclaimed when their process
exits. This control-only protocol deliberately does not map client pixels or
physical scanout memory.

Surface damage returns a monotonically increasing synchronous frame fence.
The compositor, generic display layer, and VirtIO-GPU backend preserve the
damage rectangle through redraw, backing-buffer row copy, transfer-to-host, and
resource flush. A completed fence can be queried through ABI v1.16; asynchronous
callbacks remain future work.

- UEFI GOP supplies the active mode but not EDID through the current handoff.
- There is no runtime modeset, hotplug, shared pixel buffer, frame fence, or user compositor.
- Surface geometry changes redraw the union of old and new bounds; color,
  focus, explicit damage, creation, and destruction redraw only affected bounds.
- UEFI Q35 interactive keyboard testing awaits APIC/IOAPIC input routing.
- The fixed kernel heap is temporary; high-resolution scanout buffers require
  a demand-grown page-backed heap.
