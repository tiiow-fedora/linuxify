# Linuxify

A Windows program that helps you move to Linux: it scans your PC, recommends a distro, lets you try it
safely, stages your files, and (last, and only with explicit confirmation) installs Linux.

Status: **Stages 1–2 work** (scan + readiness report, distro chooser). See `PROGRESS.md`; design choices are in `DECISIONS.md`.

```
cargo build --release
target\release\linuxify.exe scan --open
target\release\linuxify.exe choose --open
```

The scan is read-only. It writes only to `%LOCALAPPDATA%\Linuxify\reports\`.

To teach Linuxify about more programs, edit `data/app_map.json` (format: `data/SCHEMA.md`).
