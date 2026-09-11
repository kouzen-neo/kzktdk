<div align="center">
  <img src="docs/assets/app_icon.png" width="100" alt="KZKT Logo" />
  <h1>KZKT-DK (kzktdk)</h1>
  <p><b>High-Performance Native Manga & Comic Translation CLI</b></p>
  <p>A standalone command-line translation tool ported with full algorithmic fidelity from <a href="https://github.com/kouzen-neo/kzkt">KZKT Android</a>.</p>
  
  [![Release](https://img.shields.io/github/v/release/kouzen-neo/kzktdk?style=flat-square)](https://github.com/kouzen-neo/kzktdk/releases)
  [![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue?style=flat-square)](LICENSE)
  [![Rust](https://img.shields.io/badge/rust-100%25-orange?style=flat-square)](https://www.rust-lang.org)
</div>

---

## Overview

**kzktdk** (*KZKT Desktop Kit*) is a native, high-throughput command-line tool designed for the automated translation of manga, manhwa, manhua, and comics across Linux, macOS, and Windows.

It brings the complete translation pipeline of KZKT Mobile to the desktop as a fast, standalone executable:

- **Local Speech Bubble Detection**: Employs an on-device 3-stage YOLOv8 ONNX cascade model to accurately isolate speech bubbles, false-giant backgrounds, and skewed text areas.
- **Rapid OCR (NEW v0.2.0)**: PP-OCRv3 models for Japanese, English, Korean, Chinese with auto-language detection. Recognizes text at ~2 seconds per page with automatic model download.
- **Adaptive Text Inpainting**: Isolates dialogue strokes and applies morphological dilation before Telea Fast Marching inpainting, erasing original Japanese, Korean, or Chinese text while preserving paper grain, screentones, and bubble borders.
- **Multi-Provider LLM Translation**: Direct integration with Google Gemini, OpenAI (GPT), Anthropic (Claude), OpenRouter, or 100% offline local vision models running through Ollama.
- **Diamond & Elliptical Typesetting**: Uses an elliptical line-budget algorithm that naturally wraps translated text inside curved manga bubbles without overflow, combined with language-aware phonetic syllable hyphenation.
- **Editor Backend (NEW v0.2.0)**: Full metadata export/edit/render/preview workflow with JSON sidecar files, ready for GUI integration. Live bubble preview, batch rendering, and CBZ packing.
- **Batch & Comic Archive Processing**: Translates individual images (`.png`, `.jpg`, `.webp`), whole image folders, or `.cbz` / `.zip` / `.pdf` comic books with natural alphanumeric page sorting (`1, 2, ..., 9, 10`).

---

## Installation

### Prerequisites

Ensure you have the following installed on your system:
- **Rust toolchain** (`cargo` and `rustc`)
- **Git**

Verify installation:
```bash
cargo --version
git --version
```

---

### 1. Clone the Repository
```bash
git clone https://github.com/kouzen-neo/kzktdk.git
cd kzktdk
```

### 2. Build and Install

#### Option A: Universal Cargo Install (Linux, macOS, Windows)
The fastest cross-platform way to build and install `kzktdk` into your system PATH:
```bash
cargo install --path .
```
This automatically compiles the release binary and installs it to your Cargo bin folder (`~/.cargo/bin` on Linux/macOS, `%USERPROFILE%\.cargo\bin` on Windows), which is already included in your PATH.

---

#### Option B: Manual Build & Symlink

##### Linux / macOS
```bash
cargo build --release
mkdir -p ~/.local/bin
ln -sf "$PWD/target/release/kzktdk" ~/.local/bin/kzktdk
```

##### Windows (PowerShell)
```powershell
cargo build --release
Copy-Item ".\target\release\kzktdk.exe" "$HOME\.cargo\bin\"
```

---

### 3. Verification
Confirm the binary is available globally:
```bash
kzktdk --version
```

---

### Environment Variables Setup

Set your API key for your chosen translation provider:

#### Linux / macOS (`~/.bashrc` or `~/.zshrc`)
```bash
# Google Gemini (Recommended)
export GEMINI_API_KEY="AIzaSy..."

# OpenAI
export OPENAI_API_KEY="sk-proj-..."

# Anthropic Claude
export ANTHROPIC_API_KEY="sk-ant-..."
```

#### Windows (PowerShell)
```powershell
# Current session:
$env:GEMINI_API_KEY = "AIzaSy..."

# Permanent user environment variable:
[System.Environment]::SetEnvironmentVariable('GEMINI_API_KEY', 'AIzaSy...', 'User')
```

---

## ✨ Features (v0.2.0)

### 🚀 Translation Pipeline
- **Multi-Provider Support**: Gemini, OpenAI, Claude, OpenRouter, or local Ollama models
- **Translation Cache**: SQLite-based cache with blake3 hashing for instant retranslation
- **Glossary Enforcement**: JSON term mapping with strict LLM compliance checking
- **Retry & Resume**: `--retry-failed` reuses good pages, retranslates only failures
- **Progress Tracking**: JSONL streaming for real-time GUI integration

### 🔍 OCR & Detection
- **Rapid OCR (NEW)**: PP-OCRv3 models for JP/EN/KR/CN with auto-language detection (~2s/page)
- **3-Stage YOLO Cascade**: 0.28/0.18/0.10 confidence thresholds with IoU filtering
- **Freetext Detection**: Capture text outside bubbles with merge-nearby algorithm
- **Tesseract Integration**: Shell-out to external binary for 100+ languages

### 🎨 Editor Backend (NEW)
- **7 Metadata Commands**: export, edit, render, preview, show, watch, pack
- **JSON Sidecar Files**: `.kedit.json` per page + project-level metadata
- **Live Preview**: Single bubble rendering in <100ms for GUI editors
- **Batch Rendering**: Parallel rendering with `--jobs auto`
- **Transactional Editing**: `--stdin-patch` with rollback on error

### 🖋️ Font Management (NEW)
- **Font Registry**: `~/.local/share/kzktdk/fonts` with global defaults
- **Per-Bubble Override**: Custom font family, size, color, stroke, alignment
- **CJK Support**: Separate Latin + CJK font handling

### 📦 Archive Support
- **CBZ/ZIP/EPUB**: Natural sort (1,2,10) with alphanumeric awareness
- **PDF**: Input + output via Pdfium bindings (see `docs/PDFIUM.md`)
- **Batch Processing**: Parallel detection/inpainting/translation with semaphore pool

### 🛡️ Quality & Safety
- **Zero Runtime Unwrap**: All panics eliminated from production paths
- **Supply Chain Audit**: GitHub Actions workflow, EUPL-1.2 inpaint accepted
- **Dual License**: MIT OR Apache-2.0
- **43 Tests**: 20 unit + 14 CLI contract + 9 Tauri integration

---

## 🚀 Quick Start

```bash
# 1. Install
cargo install --path .

# 2. Set API key (choose one)
export GEMINI_API_KEY="AIzaSy..."        # Gemini (recommended)
export OPENAI_API_KEY="sk-proj-..."     # OpenAI
export ANTHROPIC_API_KEY="sk-ant-..."   # Claude

# 3. Translate a manga chapter
kzktdk translate chapter_01.cbz -t Indonesian

# 4. Try Rapid OCR (auto-detects language)
kzktdk translate page.jpg --ocr rapid --ocr-script auto

# 5. Use local Ollama (100% offline)
kzktdk translate chapter.cbz --provider ollama \
  --openai-base-url http://localhost:11434/v1 \
  --openai-model llama3.2-vision
```

---

## CLI Commands

### Translate Comic Archive (`.cbz` / `.zip`)
```bash
kzktdk translate "chapter_01.cbz"
```

To translate into a specific language (e.g. Indonesian):
```bash
kzktdk translate "chapter_01.cbz" -t Indonesian
```

### Translate Image Directory (Batch)

Translate an entire folder of scanned comic pages into an output directory with custom prompt rules:
```bash
kzktdk translate "./manga/chapter_01" \
  --export folder \
  -o "./output/chapter_01_id" \
  -t "Indonesian" \
  --prompt "Use a casual conversational tone suited for manga dialogue." \
  --gemini-key "AIzaSy..."
```

Or package the translated pages directly into a single `.cbz` comic book archive:
```bash
kzktdk translate "./manga/chapter_01" \
  --export cbz \
  -o "./output/chapter_01_id.cbz" \
  -t "Indonesian"
```

### Translate Single Image
```bash
kzktdk translate "page_01.jpg" -o "page_01_translated.jpg"
```

### Translate with Local Ollama (Offline)
```bash
kzktdk translate "chapter_01.cbz" \
  --provider ollama \
  --openai-base-url "http://localhost:11434/v1" \
  --openai-model "llama3.2-vision"
```

### Translate with OpenAI or OpenRouter
```bash
# OpenAI
kzktdk translate "chapter_01.cbz" \
  --provider openai \
  --openai-model "gpt-4o-mini"

# OpenRouter
kzktdk translate "chapter_01.cbz" \
  --provider openai \
  --openai-base-url "https://openrouter.ai/api/v1" \
  --openai-model "google/gemini-flash-1.5" \
  --openai-key "YOUR_OPENROUTER_KEY"
```

### Translate with Anthropic Claude
```bash
kzktdk translate "chapter_01.cbz" \
  --provider claude \
  --claude-model "claude-3-5-sonnet-20241022"
```

### Translate to PDF

PDF input is auto-detected; `--export pdf` packs translated pages into one PDF file (requires a native Pdfium library — see [docs/PDFIUM.md](docs/PDFIUM.md)):

```bash
kzktdk translate "chapter_01.cbz" --export pdf -o "chapter_01_id.pdf"
kzktdk translate "chapter_01.pdf" -o "chapter_01_id.pdf" -t Indonesian
```

### Machine-Readable Progress (for scripts / GUI)

```bash
# JSONL phases per page on stderr + final summary as JSON on stdout
kzktdk translate "./manga/chapter_01" -o out/ \
  --progress jsonl --format json --quiet 2>progress.jsonl >summary.json

# Resume: reuse good pages from a previous run, retranslate only failures
kzktdk translate "./manga/chapter_01" -o out/ --retry-failed out/

# Glossary: force fixed terms (JSON object: source -> required translation)
kzktdk translate "./manga/chapter_01" -o out/ --glossary kamus.json
```

### Exit Codes

| Code | Meaning |
| :--- | :--- |
| `0` | Success |
| `2` | Invalid data (bad metadata/patch/mask/glossary/args) |
| `1` | System error, or at least one page failed (original copied) |
| `130` | Cancelled via Ctrl-C (in-flight page finished, queue aborted) |

### Editor Backend Commands

#### Export Detections to JSON (No LLM)
```bash
kzktdk metadata export "page.png" --json "page.kedit.json"
kzktdk metadata export "chapter/" --json "project.kedit.json"
```

#### Edit Metadata (Set Text, Bbox, Style)
```bash
kzktdk metadata edit "page.kedit.json" --set "1=Hello" --bbox "1=100,50,200,80"
kzktdk metadata edit "page.kedit.json" --font-size "1=22" --text-color "1=255,0,0"
kzktdk metadata edit "page.kedit.json" --stdin-patch < patch.json  # Transactional patch
```

#### Render from Metadata (Single or Batch)
```bash
# Single page
kzktdk metadata render "page.png" --metadata "page.kedit.json" -o "rendered.jpg"

# Batch render all pages in project
kzktdk metadata render --project "project.kedit.json" --images "orig/" -o "rendered/" --jobs auto
```

#### Preview Single Bubble (Live Editor)
```bash
# Save to file
kzktdk metadata preview "page.png" --metadata "page.kedit.json" --id 1 --text "Hi" -o "preview.jpg"

# Base64 to stdout (for GUI)
kzktdk metadata preview "page.png" --metadata "page.kedit.json" --id 1 --text "Hi" --to-stdout --thumb 512
```

#### Show Metadata Table
```bash
kzktdk metadata show "page.kedit.json"
kzktdk metadata show "page.kedit.json" --id 1  # Filter by bubble ID
kzktdk metadata show "page.kedit.json" --format json  # Machine-readable
```

#### Watch & Auto Re-render
```bash
kzktdk metadata watch --project "project.kedit.json" --images "orig/" -o "rendered/"
kzktdk metadata watch --project "project.kedit.json" -o "rendered/" --once  # Render once (CI)
```

#### Pack to CBZ
```bash
kzktdk metadata pack "rendered/" -o "chapter.cbz"
```

### Font Management

```bash
kzktdk font list                        # List imported fonts
kzktdk font import "CustomFont.ttf"     # Import font to registry
kzktdk font set-default "Custom Font"   # Set global default
kzktdk font get-default                 # Show current default
kzktdk font remove "Custom Font"        # Remove from registry
```

### Inspection and Preprocessing Commands

#### Speech Bubble Detection (Draw Bounding Boxes)
```bash
kzktdk detect "page.png" -o "detected.png"
kzktdk detect "chapter/" -o "detected/" --json "detections.json"
```

#### Inpaint Only (Erase Original Text)
```bash
kzktdk inpaint "page.png" -o "cleaned.png"
```

---

## Options

Usage: `kzktdk translate [OPTIONS] <INPUT>`

| Option | Description | Default |
| :--- | :--- | :--- |
| `<INPUT>` | Path to image file (`.jpg`/`.png`/`.webp`), PDF (`.pdf`), directory, or archive (`.cbz`/`.zip`) | *(Required)* |
| `-o, --output <PATH>` | Output destination file, directory, or archive | Auto |
| `--export <FORMAT>` | Batch output format: `auto`, `cbz`, `folder`, or `pdf` | `auto` |
| `-t, --target-lang <LANG>` | Target language for dialogue translation | `English` |
| `--prompt <PROMPT>` | Custom prompt instructions or additional translation rules | - |
| `--glossary <PATH>` | JSON term map (`source -> required translation`) enforced on output | - |
| `--progress <MODE>` | Per-phase progress on stderr: `text` or `jsonl` | `text` |
| `--format <FORMAT>` | Final summary on stdout: `text` or `json` | `text` |
| `--quiet` | Human logs to stderr (keep stdout machine-clean) | off |
| `--retry-failed <DIR>` | Reuse good pages from a previous run, retranslate only failures | - |
| `--batch-size <N>` | Number of dialogue bubbles to batch per LLM translation request | `15` |
| `-p, --provider <PROVIDER>`| LLM provider: `gemini`, `openai`, `ollama`, or `claude` | `gemini` |
| `--gemini-key <KEY>` | Gemini API key (or set `GEMINI_API_KEY`) | - |
| `--gemini-model <MODEL>` | Gemini model name | `gemini-3.1-flash-lite` |
| `--openai-key <KEY>` | OpenAI API key (or set `OPENAI_API_KEY`) | - |
| `--openai-model <MODEL>` | OpenAI / Ollama model name | `gpt-4o-mini` |
| `--openai-base-url <URL>` | Base URL for OpenAI-compatible endpoints | `https://api.openai.com/v1` |
| `--claude-key <KEY>` | Anthropic Claude API key (or set `ANTHROPIC_API_KEY`) | - |
| `--claude-model <MODEL>` | Claude model name | `claude-3-5-sonnet-20241022` |
| `-f, --font <PATH>` | Comic font file (TTF format) | `fonts/Komika Axis.ttf` |
| `--cjk-font <PATH>` | Font file for CJK glyph rendering | `fonts/KosugiMaru.ttf` |
| `-m, --model <PATH>` | ONNX model file path | `models/kzkt.onnx` |
| `--ocr <ENGINE>` | OCR engine for freetext: `none`, `rapid`, `tesseract`, `vision`, `local` | `none` |
| `--ocr-script <SCRIPT>` | OCR language: `auto`, `jp`, `en`, `kr`, `cn`, `cht` (rapid only supports auto/jp/en/kr/cn) | `jp` |
| `--translate-free-text` | Detect and translate text outside bubbles (requires `--ocr`) | off |
| `--mode <MODE>` | Translation mode: `vision` (LLM extracts from image), `ocr` (OCR then translate), `auto` (OCR fallback to vision) | `vision` |

---

## Notes

- **Natural Sorting**: Directory and archive inputs are sorted by natural alphanumeric order (`1, 2, ..., 9, 10`) rather than standard lexicographical order.
- **Adaptive Inpainting**: Preserves bubble contours and screentone gradients by isolating the inner bubble area and dilating text edges before inpainting.
- **Elliptical Typesetting**: Wraps text dynamically according to manga bubble geometry to avoid margin overflow.
- **Color Inversion**: Automatically switches between dark text on light backgrounds and light text on screentone backgrounds based on localized luminance.
- **Rapid OCR**: PP-OCRv3 models (JP/EN/KR/CN + auto-detect) with session cache, ~2s per page. Models auto-download to `~/.cache/kzktdk/models/rapid/`. Traditional Chinese (cht) not yet supported.
- **Editor Backend**: Full metadata export/edit/render/preview/watch/pack workflow for GUI integration. See `kzktdk metadata --help` for details.

---

## Contributing & GUI Roadmap

We are actively welcoming open-source contributors! Current priority areas:
- **Interactive Touch / Canvas Editor**: Smooth pan/zoom canvas, draggable/resizable bubble boxes, manual mask brush, and live typesetting preview.
- **Desktop GUI**: Cross-platform desktop interface (Tauri v2 / Slint / egui) with chapter queue and visual settings.
- **Hardware Acceleration**: DirectML, CoreML, and CUDA execution providers for ONNX Runtime.

Interested in contributing? Read the complete architecture and development guidelines in [CONTRIBUTING.md](CONTRIBUTING.md).

---

## Documentation

For technical architecture, mathematical formulations, and algorithmic details, see [DOCUMENTATION.md](DOCUMENTATION.md).
GUI integrators start at [DOCUMENTATION.md §7 (GUI Contract)](DOCUMENTATION.md).
PDF library setup per OS: [docs/PDFIUM.md](docs/PDFIUM.md).
Planned GPU acceleration: [docs/GPU_ROADMAP.md](docs/GPU_ROADMAP.md).
