# Contributing to KZKT-DK

Thank you for your interest in contributing to **KZKT-DK** (*KZKT Desktop Kit*)!

KZKT-DK is a high-performance native Rust translation tool for manga, manhwa, and comics, ported with full algorithmic fidelity from [KZKT Android](https://github.com/kouzen-neo/kzkt).

We are actively opening the project to open-source contributors to expand from a high-throughput CLI and headless engine into a complete desktop suite featuring an **Interactive Touch/Canvas Editor** and a **Desktop GUI** (powered by Tauri v2).

---

## Current Roadmap & Priority Areas

### 1. Desktop GUI & Interactive Canvas Editor (Highest Priority)
The headless backend for editing ([`src/editor.rs`](file:///home/kouzen/Documents/Projects/kzktdk/src/editor.rs)) and Tauri IPC command wrappers ([`src/tauri.rs`](file:///home/kouzen/Documents/Projects/kzktdk/src/tauri.rs)) are already implemented! The primary focus is now on the frontend graphical user interface:
- **Tauri v2 Desktop Application**: Setting up the frontend desktop skeleton (e.g. Svelte or React + Vite) interfacing with `kzktdk` through Tauri command bindings (`#[cfg(feature = "tauri")]`) or headless CLI JSON streaming.
- **Visual Canvas Navigation**: Smooth panning, zooming (mouse wheel / pinch gesture), and high-DPI canvas rendering (using Konva.js, Fabric.js, or HTML5 Canvas).
- **Interactive Bounding Box Controls**:
  - Drag, resize, delete, and manually add speech bubble bounding boxes.
  - Re-order reading flow (Manga Right-to-Left vs. Western Comic Left-to-Right).
- **Touch-up & Inpaint Overrides**:
  - Quick brush/eraser to manually clear residual strokes or protect intricate artwork.
  - Selective per-bubble re-inpaint and immediate preview.
- **Live Typesetting Preview**:
  - Inline text editing with live updates.
  - Instant re-rendering with curved elliptical wrapping, font size auto-fitting, and color inversion preview.

### 2. Core Engine Enhancements
- **Hardware Acceleration (Execution Providers)**:
  - Integrate optional ONNX Runtime Execution Providers via Cargo features (`directml` on Windows, `coreml` on macOS, `cuda` / `tensorrt` on Linux). See [GPU_ROADMAP.md](docs/GPU_ROADMAP.md) for architectural plans.
- **Traditional Chinese (CHT) OCR**:
  - Training or converting a PP-OCRv3 recognition model for Traditional Chinese into ONNX format.
- **Auto-Update System for Desktop GUI**:
  - Implement `tauri-plugin-updater` v2 as detailed in [UPDATE_ROADMAP.md](docs/UPDATE_ROADMAP.md).

---

## Recommended Tech Stacks for GUI & Editor

To keep binary sizes small and cross-platform performance high, we recommend either of the following approaches:

### Option A: Tauri v2 (Recommended)
- **Backend**: Rust (`src/tauri.rs` / `src/editor.rs` enabled with `--features tauri` or via CLI sidecar protocol).
- **Frontend**: Svelte, React, or Vue paired with an interactive 2D Canvas engine:
  - [Konva.js](https://konvajs.org/) / [react-konva](https://github.com/konvajs/react-konva) or [Fabric.js](http://fabricjs.com/) for interactive bubble transforms, selection handles, and touch gestures.
- **Advantages**: Native web ergonomics for touch gestures, CSS styling, sub-50MB binary footprint, and native IPC bindings to Rust.

### Option B: Pure Native Rust GUI
- **Frameworks**: [Slint](https://slint.dev/), [egui](https://github.com/emilk/egui), or [Iced](https://github.com/iced-rs/iced).
- **Advantages**: Single-language Rust codebase, zero JavaScript runtime, and direct access to memory buffers without IPC serialization.

---

## Core Engine Architecture & API Reference

The `kzktdk` repository is organized as both a high-performance command-line application (`src/main.rs`, `src/cli/`) and a modular library (`src/lib.rs`). Any desktop GUI, IPC wrapper, or external tool can interface with `kzktdk` directly.

### Core Modules Overview

| Module | Responsibility | Key Structs & Functions |
| :--- | :--- | :--- |
| [`model`](src/model/) | YOLOv8 ONNX bubble detector cascade & model decryption | `YoloModel::new()`, `detect_bubbles(&img) -> Vec<Detection>` |
| [`inpaint`](src/inpaint/) | Dialogue stroke extraction & pure Rust Telea Fast Marching | `inpaint_image(&mut RgbImage, &[Detection])`, `inpaint_mask()` |
| [`ocr`](src/ocr.rs) | Rapid OCR (PP-OCRv3 for JP, EN, KR, CN) with auto-detect & CTC decode | `OcrEngine`, `RapidOcr`, `recognize_bubbles()`, `ensure_rec_available()` |
| [`translation`](src/translation/) | Vertical mosaic generation & Multi-provider LLM API client | `MosaicBuilder::build_mosaic()`, `Provider::translate_mosaic()` |
| [`typesetting`](src/typesetting/) | Elliptical curve wrapping, hyphenation & text rasterizer | `Typesetter::render_text()`, `Hyphenator::hyphenate()` |
| [`editor`](src/editor.rs) | Transactional editor session, bubble CRUD, re-ordering & patch state | `EditorSession`, `apply_patch()`, `render_page()`, `preview_bubble()` |
| [`font`](src/font.rs) | Font registry (`~/.local/share/kzktdk/fonts`), dynamic loading & fallback | `FontRegistry`, `get_or_download_font()`, `list_fonts()` |
| [`metadata`](src/metadata.rs) | `ComicInfo.xml` metadata generation, parsing & archive embedding | `ComicInfo`, `write_comic_info()`, `read_comic_info()` |
| [`cache`](src/cache.rs) | BLAKE3 hash caching in embedded SQLite for translation reuse | `TranslationCache`, `get_cached()`, `set_cached()` |
| [`config`](src/config.rs) | Configuration loading from TOML, CLI defaults & runtime profiles | `Config`, `load_config()`, `save_config()` |
| [`preparer`](src/preparer.rs) | Input normalization, reading direction detection & image validation | `prepare_images()`, `detect_reading_order()` |
| [`archive`](src/archive.rs) | Natural alphanumeric sorting, ZIP/CBZ unpacking & CBZ packing | `prepare_input()`, `create_cbz()`, `is_archive()` |
| [`pipeline`](src/pipeline.rs) | End-to-end pipeline orchestration (detect -> OCR -> inpaint -> typeset) | `run_pipeline()`, `TranslationContext` |
| [`tauri`](src/tauri.rs) | GUI-ready IPC command wrappers (`#[cfg(feature = "tauri")]`) | `editor_load_page`, `editor_apply_patch`, `editor_render_page` |
| [`cli`](src/cli/) | Modular CLI argument parsing (`clap`) and command execution | `Args`, `translate`, `metadata`, `detect`, `font`, `inpaint` |

### Example: Using `kzktdk` as a Core Library in UI Code

```rust
use anyhow::Result;
use image::DynamicImage;
use kzktdk::model::yolo::YoloModel;
use kzktdk::inpaint::inpaint_image;
use kzktdk::typesetting::Typesetter;

fn process_page(raw_image: DynamicImage, model_path: &std::path::Path) -> Result<()> {
    // 1. Detect speech bubbles
    let mut detector = YoloModel::new(model_path)?;
    let detections = detector.detect_bubbles(&raw_image)?;

    // 2. Erase dialogue strokes (Inpaint)
    let mut cleaned_page = raw_image.to_rgb8();
    inpaint_image(&mut cleaned_page, &detections)?;

    // 3. Typeset translated text into specific bubble box
    let font_bytes = include_bytes!("../fonts/Komika Axis.ttf");
    let bubble = &detections[0];

    Typesetter::render_text(
        &mut cleaned_page,
        "Hello, world!",
        (bubble.x1, bubble.y1, bubble.x2, bubble.y2),
        font_bytes,
        None, // Optional CJK font
    )?;

    cleaned_page.save("output.png")?;
    Ok(())
}
```

---

## Local Development Workflow

### 1. Prerequisites
Ensure you have the following installed:
- **Rust Toolchain**: `rustc` and `cargo` **1.85+** (required for Rust 2024 edition).
- **Git**
- *(Optional)* **PDFium Library**: Required for reading/exporting PDF files directly. See [docs/PDFIUM.md](docs/PDFIUM.md) for setup.
- *(Optional)* **Tesseract OCR**: If developing or testing the external Tesseract fallback backend.

### 2. Building and Testing
Clone the repository and run the build:
```bash
git clone https://github.com/kouzen-neo/kzktdk.git
cd kzktdk

# Build in debug mode
cargo build

# Run unit and integration tests
cargo test

# Check code formatting
cargo fmt --check

# Run linter
cargo clippy -- -D warnings

# (Optional) Verify Tauri feature bindings
cargo check --features tauri
```

---

## Coding Standards & Guidelines

- **Idiomatic Rust & 2024 Edition**: Follow modern Rust conventions and idioms.
- **Error Handling**: Never use `unwrap()` or `expect()` on runtime paths or inside library modules (`src/`). Return descriptive errors using `anyhow::Result`.
- **Zero-Crash / Headless Reliability**: The core library must never call `std::process::exit` or print raw text to stdout unexpectedly from within library modules, ensuring safety when integrated with GUI frameworks or external consumers.
- **Algorithmic Fidelity**: Keep core computer vision and typesetting formulas aligned with [KZKT Android](https://github.com/kouzen-neo/kzkt) and [DOCUMENTATION.md](DOCUMENTATION.md).

---

## Pull Request & Contribution Process

1. **Open an Issue / RFC First**:
   - For new features or significant refactors (such as GUI scaffolding or canvas implementations), please open an **Issue / RFC** first to align on design and architecture.
2. **Fork and Branch**:
   - Fork the repository.
   - Create a feature branch: `git checkout -b feat/touch-canvas-editor`.
3. **Code Quality**:
   - Ensure code compiles cleanly with `cargo check`.
   - Format with `cargo fmt`.
   - Ensure no regressions with `cargo test`.
4. **Submit a Pull Request**:
   - Provide a clear PR description detailing what was added or changed, including UI screenshots or screen recordings for GUI/Editor submissions.
   - Link the relevant issue (e.g. `Resolves #12`).

---

## License & Attribution

- Ported from the original Android implementation: [KZKT Android](https://github.com/kouzen-neo/kzkt).
- Dual-licensed under the **MIT OR Apache-2.0** terms.
