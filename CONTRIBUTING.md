# Contributing to KZKT-DK

Thank you for your interest in contributing to **KZKT-DK** (*KZKT Desktop Kit*)!

KZKT-DK is a high-performance native Rust translation tool for manga, manhwa, and comics, ported with full algorithmic fidelity from [KZKT Android](https://github.com/kouzen-neo/kzkt).

We are actively opening the project to open-source contributors to expand from a standalone CLI into a complete desktop suite featuring an **Interactive Touch Editor** and a **Desktop GUI**.

---

## Current Roadmap & Priority Areas

### 1. Interactive Touch / Canvas Editor (High Priority)
A visual canvas editor designed for desktop mouse, touch screens, and stylus devices:
- **Canvas Navigation**: Smooth pan, zoom (mouse wheel and pinch-to-zoom), and high-DPI image rendering.
- **Bounding Box Manipulation**:
  - Drag, resize, delete, and add new speech bubble bounding boxes.
  - Re-order bubble reading flow (e.g. Manga right-to-left vs. Comic left-to-right).
- **Manual Mask & Inpaint Overrides**:
  - Quick brush/eraser to manually clear residual kanji strokes or protect delicate artwork.
  - Selective re-inpaint per individual bubble.
- **Live Typesetting Preview**:
  - Inline text editor to modify machine-translated text.
  - Instant re-rendering with curved elliptical wrapping, font size recalculation, and color inversion preview.

### 2. Desktop GUI (High Priority)
A modern, cross-platform graphical user interface (Linux, macOS, Windows):
- **Project / Chapter Manager**: Drag-and-drop support for comic archives (`.cbz`, `.zip`) and raw folder scans.
- **Batch Processing Queue**: Visual progress bars showing current page, bubble counts, and LLM query state.
- **Settings Dashboard**: Simple UI to input and safely store API keys (Gemini, OpenAI, Claude) and configure local Ollama models.
- **Dual-Pane Comparison**: Side-by-side view of original raw scan vs. cleaned/translated page.

### 3. Core Engine Enhancements
- **Hardware Acceleration**: Enable optional ONNX Runtime Execution Providers (`DirectML` on Windows, `CoreML` on macOS, `CUDA`/`TensorRT` on Linux).
- **OCR / Offline Fallback**: Optional local OCR engines (e.g. Manga-OCR) for offline translation workflows without vision LLMs.

---

## Recommended Tech Stacks for GUI & Editor

To keep binary sizes small and cross-platform performance high, we recommend either of the following approaches:

### Option A: Tauri v2 (Recommended)
- **Backend**: Rust (`crates/kzktdk-gui` or workspace crate connecting directly to `kzktdk` core).
- **Frontend**: Svelte, React, or Vue paired with an interactive 2D Canvas engine:
  - [Konva.js](https://konvajs.org/) / [react-konva](https://github.com/konvajs/react-konva) or [Fabric.js](http://fabricjs.com/) for interactive bubble transforms, selection handles, and touch gestures.
- **Advantages**: Native web ergonomics for touch gestures, CSS styling, sub-50MB binary footprint, and native IPC bindings to Rust.

### Option B: Pure Native Rust GUI
- **Frameworks**: [Slint](https://slint.dev/), [egui](https://github.com/emilk/egui), or [Iced](https://github.com/iced-rs/iced).
- **Advantages**: Single-language Rust codebase, zero JavaScript runtime, and direct access to memory buffers without IPC serialization.

---

## Core Engine Architecture & API Reference

The `kzktdk` repository is organized as both a reusable library (`src/lib.rs`) and a command-line application (`src/main.rs`). Any GUI or external tool can import `kzktdk` directly.

### Core Modules Overview

| Module | Responsibility | Key Structs & Functions |
| :--- | :--- | :--- |
| [`model`](src/model/) | YOLOv8 ONNX bubble detector & model decryption | `YoloModel::new()`, `detect_bubbles(&img) -> Vec<Detection>` |
| [`inpaint`](src/inpaint/) | Dialogue stroke extraction & Telea Fast Marching | `inpaint_image(&mut RgbImage, &[Detection])` |
| [`translation`](src/translation/) | Vertical mosaic generation & Multi-provider LLM API | `MosaicBuilder::build_mosaic()`, `Provider::translate_mosaic()` |
| [`typesetting`](src/typesetting/) | Elliptical curve wrapping, hyphenation & text rasterizer | `Typesetter::render_text()`, `Hyphenator::hyphenate()` |
| [`archive`](src/archive/) | Natural page sorting, archive unpacking & CBZ export | `prepare_input()`, `create_cbz()` |

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
- **Rust Toolchain**: `rustc` and `cargo` (1.75+)
- **Git**

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
```

---

## Pull Request & Contribution Process

1. **Open an Issue First**:
   - For new features or significant refactors (such as adding a GUI workspace or canvas framework), please open an **Issue / RFC** first to discuss the design and architecture.
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
- Contributions made to this repository will be licensed under the project's open-source terms.
