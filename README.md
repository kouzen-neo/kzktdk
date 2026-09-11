<div align="center">
  <img src="docs/assets/app_icon.png" width="100" alt="KZKT Logo" />
  <h1>KZKT-DK (kzktdk)</h1>
  <p><b>High-Performance Native Manga & Comic Translation CLI</b></p>
  <p>A standalone, native Rust translation toolkit ported with full algorithmic fidelity from <a href="https://github.com/kouzen-neo/kzkt">KZKT Android</a>.</p>
  
  [![Release](https://img.shields.io/github/v/release/kouzen-neo/kzktdk?style=flat-square)](https://github.com/kouzen-neo/kzktdk/releases)
  [![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue?style=flat-square)](LICENSE)
  [![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?style=flat-square)](https://www.rust-lang.org)
</div>

---

## Features

### Translation Pipeline
- **Multi-Provider Support**: Gemini, OpenAI, Claude, OpenRouter, or local Ollama models.
- **Translation Cache**: SQLite-based cache with blake3 hashing for instant retranslation.
- **Glossary Enforcement**: JSON term mapping with strict LLM compliance checking.
- **Retry & Resume**: `--retry-failed` reuses good pages, retranslates only failures.
- **Progress Tracking**: JSONL streaming for real-time GUI and script integration.

### OCR & Text Recognition
- **Rapid OCR**: PP-OCRv3 models for Japanese, English, Korean, Chinese with auto-language detection (~2s/page).
- **3-Stage YOLO Cascade**: 0.28/0.18/0.10 confidence thresholds with IoU filtering.
- **Freetext Detection**: Capture text outside bubbles with merge-nearby clustering.
- **Tesseract Integration**: Shell-out to external binary for 100+ languages.

### Inpainting & Typesetting
- **Adaptive Inpainting**: Pure Rust Telea FMM inpainting preserving screentones and paper grain.
- **Elliptical Typesetting**: Geometry-aware text wrapping contoured to manga bubble curvature.

### Editor Backend & Formats
- **Metadata Workflows**: 7 commands (`export`, `edit`, `render`, `preview`, `show`, `watch`, `pack`).
- **JSON Sidecar Files**: `.kedit.json` per page with transactional editing and atomic rollback.
- **Font Management**: Global font registry at `~/.local/share/kzktdk/fonts` with per-bubble overrides.
- **Format Support**: CBZ, ZIP, EPUB, PDF, and image folders with natural alphanumeric sorting (`1, 2, ..., 9, 10`).

---

## Installation

### Quick Install (No Compilation Required)

Install pre-compiled standalone binaries and required assets in one step:

#### Linux & macOS
```bash
curl -fsSL https://raw.githubusercontent.com/kouzen-neo/kzktdk/master/install.sh | bash
```

#### Windows (PowerShell)
```powershell
irm https://raw.githubusercontent.com/kouzen-neo/kzktdk/master/install.ps1 | iex
```

#### Uninstall
- **Linux & macOS**: `curl -fsSL https://raw.githubusercontent.com/kouzen-neo/kzktdk/master/uninstall.sh | bash`
- **Windows**: `irm https://raw.githubusercontent.com/kouzen-neo/kzktdk/master/uninstall.ps1 | iex`

---

### Build from Source (Rust 1.85+)
```bash
cargo install --path .
```
*For development guidelines and building from source, see [CONTRIBUTING.md](CONTRIBUTING.md).*

---

## API Key Configuration

Set the API key for your chosen translation provider:

```bash
# Google Gemini (Default / Recommended)
export GEMINI_API_KEY="AIzaSy..."

# OpenAI
export OPENAI_API_KEY="sk-proj-..."

# Anthropic Claude
export ANTHROPIC_API_KEY="sk-ant-..."
```

*(On Windows PowerShell, use `$env:GEMINI_API_KEY = "AIzaSy..."`)*

---

## CLI Commands

### 1. `kzktdk translate` — Translate Manga & Comics

#### Basic Usage
```bash
# Translate a comic archive to Indonesian (default output: chapter_01_translated.cbz)
kzktdk translate chapter_01.cbz -t Indonesian

# Translate a single image file
kzktdk translate page_01.jpg -o page_01_id.jpg -t Indonesian

# Translate an entire folder and export as a new CBZ archive
kzktdk translate ./scans/chapter_01/ -o chapter_01.cbz --export cbz -t English

# Translate a PDF file (requires libpdfium)
kzktdk translate volume_01.pdf -o volume_01_id.pdf -t Indonesian
```

#### Provider Options
```bash
# Google Gemini (Default)
kzktdk translate chapter.cbz --provider gemini --gemini-model gemini-3.1-flash-lite

# OpenAI
kzktdk translate chapter.cbz --provider openai --openai-model gpt-4o-mini

# Anthropic Claude
kzktdk translate chapter.cbz --provider claude --claude-model claude-3-5-sonnet-20241022

# 100% Offline Local Translation via Ollama
kzktdk translate chapter.cbz --provider ollama \
  --openai-base-url http://localhost:11434/v1 \
  --openai-model llama3.2-vision
```

#### OCR & Freetext Detection
```bash
# Auto-detect language with Rapid OCR (JP / EN / KR / CN)
kzktdk translate chapter.cbz --ocr rapid --ocr-script auto

# Capture text outside bubbles (freetext)
kzktdk translate chapter.cbz --ocr rapid --translate-free-text

# External Tesseract engine (for 100+ other languages)
kzktdk translate page.jpg --ocr tesseract --ocr-script jpn
```

#### Reliability & Automation
```bash
# Resume interrupted jobs: retranslates only failed pages from a prior run
kzktdk translate ./chapter_01/ -o out/ --retry-failed out/

# Enforce strict terminology via JSON glossary mapping
kzktdk translate chapter.cbz -o out.cbz --glossary glossary.json

# Stream JSONL progress for GUI / scripts without polluting stdout
kzktdk translate ./chapter_01/ -o out/ --progress jsonl --format json --quiet 2>progress.jsonl >summary.json
```

---

### 2. `kzktdk detect` — Visualize Speech Bubbles

Detect dialogue bubbles and output bounding box previews or JSON coordinates:

```bash
# Save detection visualization with colored bounding boxes
kzktdk detect page.jpg -o preview_boxes.png

# Export detection coordinates to JSON
kzktdk detect ./chapter_01/ -o ./detected/ --json detections.json
```

---

### 3. `kzktdk inpaint` — Clean Bubbles Without Translating

Erase Japanese/Korean/Chinese text strokes while preserving clean bubble backgrounds:

```bash
kzktdk inpaint page.jpg -o cleaned_page.jpg
```

---

### 4. `kzktdk metadata` — Editor & Sidecar Workflows

Perform decoupled editing workflows using `.kedit.json` sidecar files:

```bash
# Export: Detect bubbles and generate metadata without LLM calls
kzktdk metadata export page.jpg --json page.kedit.json
kzktdk metadata export ./chapter_01/ --json project.kedit.json

# Edit: Modify translations, style, or bounding boxes
kzktdk metadata edit page.kedit.json --set "1=Hello World" --font-size "1=24"
kzktdk metadata edit page.kedit.json --stdin-patch < patch.json   # Transactional batch edit

# Render: Inpaint and typeset from metadata without querying the LLM
kzktdk metadata render page.jpg --metadata page.kedit.json -o rendered.jpg
kzktdk metadata render --project project.kedit.json --images ./raw/ -o ./rendered/ --jobs auto

# Preview: Render a single bubble in <100ms (ideal for live GUI canvas)
kzktdk metadata preview page.jpg --metadata page.kedit.json --id 1 --text "Preview" -o bubble.jpg

# Inspect: Display bubble status table
kzktdk metadata show page.kedit.json

# Watch: Continuously re-render dirty pages when metadata changes
kzktdk metadata watch --project project.kedit.json --images ./raw/ -o ./rendered/

# Pack: Package a rendered image folder into a CBZ comic archive
kzktdk metadata pack ./rendered/ -o chapter_final.cbz
```

---

### 5. `kzktdk font` — Font Registry

Manage comic fonts stored in `~/.local/share/kzktdk/fonts`:

```bash
kzktdk font list                        # List installed fonts
kzktdk font import "WildWords.ttf"      # Import font into registry
kzktdk font set-default "Wild Words"    # Set global default Latin font
kzktdk font get-default                 # Display current default fonts
kzktdk font remove "OldFont"            # Remove font from registry
```

---

## Command-Line Options Reference

Run `kzktdk translate --help` for complete details. Common flags:

| Flag | Description | Default |
|:---|:---|:---|
| `<INPUT>` | Path to image file, folder, `.cbz`, `.zip`, or `.pdf` | *(Required)* |
| `-o, --output <PATH>` | Output destination file or directory | Auto |
| `-t, --target-lang <LANG>` | Target language for dialogue translation | `English` |
| `--prompt <PROMPT>` | Custom prompt instructions (e.g. style, context) | - |
| `-p, --provider <NAME>` | LLM provider: `gemini`, `openai`, `claude`, `ollama` | `gemini` |
| `--ocr <ENGINE>` | OCR engine: `none`, `rapid`, `tesseract`, `vision` | `none` |
| `--ocr-script <SCRIPT>` | OCR script: `auto`, `jp`, `en`, `kr`, `cn` | `jp` |
| `--translate-free-text` | Detect and translate text outside dialogue bubbles | `false` |
| `--glossary <FILE>` | Path to JSON dictionary mapping terms strictly | - |
| `--retry-failed <DIR>` | Re-process only failed pages from a prior run directory | - |
| `--export <FORMAT>` | Batch output format: `auto`, `cbz`, `folder`, `pdf` | `auto` |
| `--progress <MODE>` | Progress logging to stderr: `text` or `jsonl` | `text` |
| `--format <FORMAT>` | Summary format to stdout: `text` or `json` | `text` |
| `--quiet` | Silence progress logs to keep stdout clean for piping | `false` |

---

## Further Documentation

- **[DOCUMENTATION.md](DOCUMENTATION.md)**: Full technical architecture, mathematical formulations, inpainting algorithms, exit codes (§9), and the GUI integration contract (§7).
- **[CONTRIBUTING.md](CONTRIBUTING.md)**: Coding standards, invariants, and local development testing.
- **[docs/PDFIUM.md](docs/PDFIUM.md)**: Pdfium setup guide for reading and writing PDF comics.
- **[docs/GPU_ROADMAP.md](docs/GPU_ROADMAP.md)**: Hardware acceleration roadmap (DirectML, CoreML, CUDA).
- **[CHANGELOG.md](CHANGELOG.md)**: Release history and version notes.
