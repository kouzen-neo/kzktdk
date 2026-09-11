# KZKT-DK Technical Architecture & Specifications

This document details the internal design, mathematical formulations, and engineering decisions behind `kzktdk`.

---

## 1. System Overview

`kzktdk` is a high-performance native Rust translation pipeline for comic and manga imagery. It replicates the core processing algorithms of [KZKT Android](https://github.com/kouzen-neo/kzkt) without relying on Python or OpenCV C++ libraries.

```text
               Input Image / Folder / CBZ
                           │
                           ▼
          ┌──────────────────────────────────┐
          │  YOLOv8 Speech Bubble Detector   │ (ONNX Runtime)
          └──────────────────────────────────┘
                           │
                 [Detections: BBoxes]
                           │
             ┌─────────────┴─────────────┐
             ▼                           ▼
    ┌─────────────────┐         ┌─────────────────┐
    │  Mosaic Builder │         │  Interior Mask  │
    └─────────────────┘         │  & Dilation     │
             │                  └─────────────────┘
    [Vertical Mosaic]                    │
             │                           ▼
             ▼                  ┌─────────────────┐
    ┌─────────────────┐         │  Telea Inpaint  │
    │  LLM Translator │         │  (Pure Rust)    │
    └─────────────────┘         └─────────────────┘
             │                           │
    [Text Translations]          [Cleaned Image]
             │                           │
             └─────────────┬─────────────┘
                           ▼
          ┌──────────────────────────────────┐
          │   Diamond / Elliptical Typesetter│
          │   & Phonetic Syllable Wrap       │
          └──────────────────────────────────┘
                           │
                           ▼
              Translated Output (Image/CBZ)
```

---

## 2. Speech Bubble Detection (YOLOv8 Cascade)

### 2.1 Inference Pipeline
- Uses Microsoft ONNX Runtime (`ort` v2.0) with multi-threaded CPU execution.
- Input: Image resized to `640x640` with letterbox padding.
- Normalization: RGB values normalized to $[0.0, 1.0]$.
- Tensor shape: `[1, 3, 640, 640]` in `NCHW` planar order.

### 2.2 3-Stage Confidence Cascade
To capture faint sketched bubbles without introducing false positives, a three-stage cascade filter is employed:

$$\begin{aligned}
\text{Stage 1 (Primary):} \quad & \text{Confidence} \ge 0.28, \; \text{IoU Threshold} = 0.45 \\
\text{Stage 2 (Secondary):} \quad & \text{Confidence} \ge 0.18, \; \text{IoU Threshold} = 0.55 \\
\text{Stage 3 (Sensitive):} \quad & \text{Confidence} \ge 0.10, \; \text{IoU Threshold} = 0.65
\end{aligned}$$

### 2.3 Geometric False-Positive Filtering
- **Aspect Ratio Guard**: Eliminates bounding boxes where width-to-height or height-to-width ratio exceeds $12:1$.
- **False-Giant Suppression**: Discards any detected box exceeding $92\%$ of total page area, preventing full-page false detections from suppressing legitimate interior bubbles.

---

## 3. Mask Generation & Telea Fast Marching Inpainting

### 3.1 Bubble Interior Isolation
To prevent destroying black manga speech bubble borders or background screentones, text extraction is constrained to the interior region:
1. **Luminance Calculation**:
   $$\text{Luminance}(R, G, B) = 0.299R + 0.587G + 0.114B$$
2. **Background Classification**:
   - Median background luminance inside the bubble determines whether the bubble is light (standard dialogue) or dark (screentone / inverted).
3. **Contrast Masking**:
   - For light bubbles, pixels darker than the background threshold by a margin $\Delta L \ge 45$ are marked as text strokes.
   - For dark bubbles, pixels significantly lighter than the background are marked.

### 3.2 Anti-Aliasing & Stroke Dilation
Text rasterization creates semi-transparent anti-aliased edge pixels. If left unmasked, these leave noticeable gray halos after inpainting.
- A circular structuring element of radius $r = 2$ dilates the text stroke mask.
- Morphological dilation ensures all sub-pixel fringe artifacts are enclosed within the inpainting domain.

### 3.3 Telea Fast Marching Method (Pure Rust)
The inpainting engine uses the Alexandru Telea algorithm implemented in pure Rust (`inpaint` crate):
- Computes the Eikonal equation using Fast Marching Method (FMM) to propagate color gradients from the boundary inward.
- Preserves paper texture, subtle gradients, and background screentones seamlessly without blur or discoloration.

---

## 4. Diamond & Elliptical Typesetting Engine

Speech bubbles in comics and manga are primarily oval or elliptical. Standard rectangular wrapping creates ugly line breaks or overflows at the top and bottom corners.

### 4.1 Elliptical Width Function
For a bubble of usable width $W_{\max}$ and total lines $N$:
$$\hat{y}_i = \frac{2i + 1}{N} - 1, \quad \hat{y}_i \in [-1, 1]$$
$$\text{Scale}(i) = \max\left(0.82, \; \sqrt{\max\left(0.20, \; 1.0 - 0.40 \cdot \hat{y}_i^2\right)}\right)$$
$$W_i = W_{\max} \cdot \text{Scale}(i)$$

This creates a diamond/elliptical bounding contour that naturally fills the center of the bubble wider and tapers the top and bottom lines.

### 4.2 Phonetic Syllable Hyphenator
To prevent awkward breaks (e.g. `d-` on one line and `iatas` on the next), the layout engine incorporates an Indonesian and Latin phonetic syllabification module:
- Implements standard phonotactic and EYD syllabification rules.
- Identifies consonant clusters (`ng`, `ny`, `sy`, `kh`), diphthongs (`ai`, `au`, `oi`), and syllable boundaries ($V$-$CV$, $VC$-$CV$, $V$-$V$).
- Generates valid hyphenation candidates when long words exceed line limits.

---

## 5. Model Decryption Specification

The bundled model (`models/kzkt.dat`) is an obfuscated binary from the KZKT distribution.

- **Algorithm**: Byte-wise symmetric stream XOR with key `0x5A` (`90` decimal).
- **Validation**:
  - The first byte of an ONNX Protobuf payload must match `0x08` (Protobuf tag 1 for `ir_version`).
  - Decrypted header is verified before writing the full model to `models/kzkt.onnx`.

---

## 6. PDF Support (Pdfium)
PDF input/output is implemented in `src/archive.rs` via the `pdfium-render` crate (dynamic binding, no static bundling).

- **Input**: `prepare_input()` accepts `.pdf` → pages are rendered to PNG (`page_0001.png`, target width 1600px) in a `TempDir`, then flow through the normal batch pipeline (`PreparedInput::Batch { is_pdf: true, ... }`).
- **Output**: `translate --export pdf` (or auto when input is PDF / `-o` ends in `.pdf`) packs translated pages with `create_pdf()` — one full-page image per page, page size = image pixels as points, JPEG quality 90. Default name: `<stem>_translated.pdf`.
- **Native library resolution** (`bind_pdfium()`): `PDFIUM_LIB_PATH` env var → system library (`libpdfium.so` / `libpdfium.dylib` / `pdfium.dll`). Without it, PDF commands fail with a descriptive error (exit 1), never a panic.
- Setup per OS + verification: `docs/PDFIUM.md`, `./scripts/verify_pdf.sh` (prints `PDF_OK` / `PDF_SKIP`).
- `detect` / `metadata export` also accept PDF input (pages become batch images).

---

## 7. GUI Contract (Machine Backend)

Frontend (Tauri v2 + Svelte + Konva.js) drives `kzktdk` without parsing human logs. Conventions:

- **stdout** carries ONLY machine data (JSON / JSON arrays / single-line base64 PNG).
- **stderr** carries ALL human logs, progress JSONL, and warnings — always, and especially under `--quiet` or `--format json` / `--progress jsonl`.
- **Exit codes**: `0` ok · `2` invalid data (bad metadata, bad patch, bad glossary, bad mask, failed validation) · `1` system/pipeline errors · `130` cancelled via Ctrl-C.

### 7.1 Schemas

`Bubble` (`src/metadata.rs`): `{id, bbox:[x1,y1,x2,y2], conf, translated, bg_color?, style?, edited, raw_text?, mask_path?}`. `translated == ""` skips rendering; `"SKIP"` (case-insensitive) skips inpaint + typeset. `mask_path` points to a grayscale brush mask (white = inpaint) sized exactly to the bubble crop.

`BubbleStyle` (all optional): `{font_family?, font_size?, text_color?[r,g,b], stroke_color?, align? (left|center|right), is_bold?, is_italic?}`.

`PageEditData` (`.kedit.json`): `{version:1, page, width, height, target_lang, prompt_sig, bubbles[], ocr_engine?, order?}`. `order` lists bubble IDs in reading order; absent = stored order. Old files without `order`/`mask_path`/`raw_text` still load (`serde default`).

`Project` (`project.kedit.json`): `{version:1, pages[], target_lang?, ocr_engine?}`.

`EditPatch` (for `edit --stdin-patch` and `tauri::editor_apply_patch`): `{set[], bbox[], add[], delete[], font_family[], font_size[], text_color[], stroke_color[], align[], edited[], bg_color[], conf[], clear_style[], raw_text[], replace[], apply_style?, bold[], italic[], order? ("1,ft1,2"), mask_path[]}`. Applied transactionally: any error aborts with NO write.

### 7.2 Command surface for GUIs

| Command | Machine output |
|---|---|
| `metadata show --format json [--id ID]` | Page/project/bubble JSON to stdout |
| `metadata validate --format json` | `ValidationReport{valid,kind,errors[{id?,code,msg}],meta}` to stdout; exit 2 when invalid. Codes: `duplicate_id`, `invalid_bbox`, `out_of_bounds`, `unknown_order_id` |
| `metadata edit --stdin-patch` | Reads `EditPatch` JSON from stdin; stdout `{"ok":true,"logs":[]}` or stderr `{"ok":false,"errors":[],"logs":[]}` + exit 2 (no write) |
| `metadata preview --to-stdout [--thumb N]` | Single-line base64 PNG to stdout; logs to stderr |
| `metadata render --progress jsonl` | Per page `{"idx","total","page","out","ok","ms"}` + final `{"done":true,"ok_count","fail_count"}` to stderr |
| `metadata watch --project --images -o [--once]` | Same JSONL as render, only dirty pages (bubble-hash compare) |
| `metadata mask --id ID --from mask.png` / `--clear` | Validates mask size = bubble crop; exit 2 on mismatch |
| `metadata pack --format json` | `{"ok","output","count","files[]}` to stdout |
| `metadata export --reading-order r2l\|l2r\|off` | Fills `order` from geometry (default `r2l`) |
| `detect --format json [--quiet]` | `[{page,width,height,bubbles[{id,bbox,conf}],freetext[{id,bbox}]}]` to stdout |
| `translate --progress jsonl --format json --quiet --glossary G --retry-failed DIR` | Per-phase `{"idx","total","page","phase":"detect_end\|translate_end\|render_end"}` to stderr; final `{"done":true,...}` to stderr; summary `{"ok","files[{idx,file,out,ok,skipped,error}]","ok_count","fail_count","skipped_count","glossary_hits","glossary_misses","output"}` to stdout with `--format json`; exit 1 when any page failed |
| `font list/get-default --format json` | Registry JSON to stdout |

### 7.3 Full session example

```bash
kzktdk metadata export page.png --json p.kedit.json
kzktdk metadata show p.kedit.json --format json
echo '{"set":["1=Halo"]}' | kzktdk metadata edit p.kedit.json --stdin-patch
kzktdk metadata preview page.png --metadata p.kedit.json --id 1 --text "Halo" --to-stdout --thumb 512 | base64 -d > prev.png
kzktdk metadata render page.png --metadata p.kedit.json -o out.jpg --progress jsonl
kzktdk metadata pack ./rendered -o chapter.cbz --format json
```

### 7.4 Rust library for Tauri (`src/editor.rs`, `src/tauri.rs`)

`EditorSession::new(fonts)` / `open_model(model, fonts)` (YOLO loaded once) + `detect_bubbles`, `export_page`, `load_page`, `render_page`, `preview_bubble`, helpers `thumbnail`, `png_base64`, `bubbles_hash`. Feature `tauri` gates `src/tauri.rs` wrappers (`editor_load_page`, `editor_apply_patch`, `editor_render_page`, `editor_preview_bubble`, `editor_detect`, `editor_pack_chapter`) returning `Result<T, String>` — attach `#[tauri::command]` in the GUI project. No `println!`, no `process::exit` inside the library.

---

## 8. OCR Support (Rapid OCR 5-Language Implementation)

### 8.1 Rapid OCR (PP-OCRv3 ONNX)

**Implemented**: Full text recognition for 5 languages with auto-detection.

#### Supported Languages
- **Japanese** (`--ocr-script jp`): PP-OCRv3 JP model (3.6 MB)
- **English** (`--ocr-script en`): PP-OCRv3 EN model (9.0 MB)
- **Korean** (`--ocr-script kr`): PP-OCRv3 KR model (3.3 MB)
- **Chinese Simplified** (`--ocr-script cn`): PP-OCRv3 CN model (10.7 MB)
- **Auto-detect** (`--ocr-script auto`): Trial-decode with 2-3 sample bubbles, picks best confidence (threshold 0.3)

#### Architecture
- **Detection**: PP-OCRv3 detection model (2.4 MB, shared across languages)
- **Recognition**: CTC decode with blank collapse, per-language character dictionaries
- **Session Cache**: Models loaded once per process (`Arc<Mutex<Session>>` via `OnceLock`)
- **Model Download**: Auto-fetched from HuggingFace/PaddleOCR GitHub, cached `~/.cache/kzktdk/models/rapid/`

#### Performance
- **Speed**: ~2s per page (down from 4.7s without session cache, 57% faster)
- **Batch**: 5 pages in 10.4s (~2s/page average)

#### Traditional Chinese (cht)
Not yet implemented (no ONNX available from PaddleOCR). Error message directs to `--ocr-script cn` or `--ocr tesseract`.

#### Usage
```bash
# Japanese (default)
kzktdk translate "page.jpg" --ocr rapid --ocr-script jp

# Auto-detect (English → Japanese → Korean → Chinese)
kzktdk translate "page.jpg" --ocr rapid --ocr-script auto

# Korean
kzktdk translate "page.jpg" --ocr rapid --ocr-script kr
```

### 8.2 Tesseract OCR (External Binary)

Shells out to `tesseract` binary (graceful fallback when absent). Supports 100+ languages via Tesseract's language packs.

```bash
kzktdk translate "page.jpg" --ocr tesseract --ocr-script jpn
```

### 8.3 Vision-based OCR (LLM)

Default mode. No local OCR models required — LLM extracts text from mosaic image directly.

```bash
kzktdk translate "page.jpg" --mode vision  # Default
```

### 8.4 Manga OCR (Deprecated)

`--ocr manga` now shows deprecation warning and falls back to noop (stub removed). Use `--ocr rapid` or `--ocr tesseract` instead.
