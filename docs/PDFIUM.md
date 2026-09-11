# Pdfium Setup (PDF in/out)

`kzktdk` reads/writes PDF via the [`pdfium-render`](https://crates.io/crates/pdfium-render)
crate (currently 0.9), which **dynamically binds** to a native Pdfium library.
The library is **not bundled** — install it once per machine. Without it, every
PDF path (`translate` PDF in/out, `detect`/`metadata export` on `.pdf`) fails
gracefully with exit 1 plus an install hint (never a panic); all other
commands work normally.

Resolution order in `bind_pdfium()` (`src/archive.rs`):

1. `PDFIUM_LIB_PATH` env var (full file path, recommended),
2. system library found by the OS loader.

## Per-OS library file

| OS      | File searched       | Typical location                                  |
|---------|---------------------|---------------------------------------------------|
| Linux   | `libpdfium.so`      | `/usr/lib/`, `/usr/local/lib/` (via `ldconfig`)  |
| macOS   | `libpdfium.dylib`   | `/opt/homebrew/lib/`, `/usr/local/lib/`          |
| Windows | `pdfium.dll`        | next to `kzktdk.exe`, or `PATH`                   |

## Where to download builds

Prebuilt binaries per OS/arch are published at
**`bblanchon/pdfium-binaries`** (GitHub Releases, `pdfium-<os>-<arch>.tgz`).
Pick the archive matching your OS and CPU, extract the library file.

> Third-party builds: verify the checksum/source against your own trust
> policy before placing the file in a system directory.

## Setup

Linux:

```bash
# option A — user-local, no root
mkdir -p ~/.local/lib && cp libpdfium.so ~/.local/lib/
echo 'export PDFIUM_LIB_PATH="$HOME/.local/lib/libpdfium.so"' >> ~/.bashrc
# option B — system-wide
sudo cp libpdfium.so /usr/local/lib/ && sudo ldconfig
```

macOS:

```bash
cp libpdfium.dylib /opt/homebrew/lib/   # Apple Silicon
# or: export PDFIUM_LIB_PATH="$HOME/lib/libpdfium.dylib"
```

Windows (PowerShell):

```powershell
Copy-Item pdfium.dll (Join-Path (Get-Location) 'pdfium.dll')
# or machine-wide:
[Environment]::SetEnvironmentVariable('PDFIUM_LIB_PATH', 'C:\libs\pdfium.dll', 'User')
```

## Verify

```bash
./scripts/verify_pdf.sh
# PDF_OK   -> round-trip create/extract/create passed (2 synthetic PNGs)
# PDF_SKIP -> library absent; PDF commands exit 1 with a hint
```

Manual check (expects exit 1 + message when the library is absent):

```bash
kzktdk metadata export doc.pdf --json x.json -m models/kzkt.onnx
```

## Notes for packagers / CI

- CI jobs that need PDF coverage must install the library first, then
  `cargo test pdf_roundtrip_or_skip` runs the real round-trip; otherwise it
  soft-skips and the suite stays green.
- GUI installers (future Tauri app) should bundle `pdfium.dll` /
  `libpdfium.dylib` / `libpdfium.so` next to the binary so users need no setup.

See also: `DOCUMENTATION.md` §6 (PDF Support), `README.md` (PDF section).
