# Vantara Filesystem Layout

Vantara exposes this canonical root namespace:

```text
/
├── System/
├── Apps/
├── Users/
├── Config/
├── Data/
├── Cache/
├── Logs/
├── Runtime/
├── Devices/
├── Temp/
├── Packages/
├── Boot/
└── Volumes/
```

`/Data` is the canonical VANTFS persistence mount and `/Devices` is the
canonical devfs mount. Existing software remains compatible through `/persist`
and `/dev`, which resolve to the same respective backends.

`/Apps` exposes the same generated application registry as `/bin`. `/Temp`
shares the writable in-memory store with `/tmp`. The remaining directories are
reserved root namespaces and begin empty.
