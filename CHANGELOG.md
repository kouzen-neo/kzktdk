# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned
- Tauri v2 GUI skeleton (Svelte + Konva.js)
- Hardware acceleration (DirectML/CoreML/CUDA)
- Traditional Chinese (cht) OCR model conversion

---

## [0.2.1] - 2026-09-11

**Performance & Code Quality**: 3x bubble detection acceleration, HTTP connection pooling, OCR allocation optimizations, and metadata god-function deconstruction.

### Performance & Optimization
- **YOLO 3x Detection Acceleration** (`src/model/yolo.rs`): Separated ONNX `session.run` execution into `forward(&mut self, prepared)` and multi-scale decoding into `decode_detections(...)`. Bubble detection now performs **one** forward pass per image instead of three, yielding ~3x speedup.
- **HTTP Connection Pooling** (`src/translation/mod.rs`): Replaced per-request `reqwest::Client::new()` instantiation in `translate_mosaic_raw` and `translate_text` with a shared static connection pool via `OnceLock<reqwest::Client>` (120s request timeout, 20s connect timeout).
- **OCR Heap Allocation Reduction** (`src/ocr.rs`): Replaced 2D vector allocation `vec![vec![false; w]; h]` in heatmap text-box detection with a flat 1D vector `vec![false; w * h]`, eliminating ~1,280 heap allocations per page.

### Refactoring & Architecture
- **Metadata God-Function Deconstruction** (`src/cli/metadata.rs`): Split the monolithic 1,442-line `run` function into 9 isolated, testable helpers: `run_export`, `run_render`, `run_edit`, `run_validate`, `run_preview`, `run_show`, `run_pack`, `run_mask`, `run_watch`.
- **CLI Argument Extraction** (`src/cli/args.rs`, `src/main.rs`, `src/cli/translate.rs`): Extracted 34 inlined fields from `Commands::Translate` into a dedicated `TranslateArgs` struct and simplified 75 lines of manual argument unpacking in `main.rs` to a single clean dispatch line.
- **Luminance Magic Number Centralization** (`src/inpaint/mod.rs`): Extracted ITU-R BT.601 luminance coefficients (`LUM_R`, `LUM_G`, `LUM_B`) and inpainting thresholds (`LIGHT_BUBBLE_MEAN_LUM_THRESHOLD`, `LIGHT_BG_MIN_LUM`, `LIGHT_TEXT_MAX_LUM`, `DARK_BG_MAX_LUM`, `DARK_TEXT_MIN_LUM`, `TELEA_INPAINT_RADIUS`, `TEXT_DILATION_RADIUS`, `INSET_FRACTION`) into named constants with inline helpers `rgb_to_lum_f64` and `rgb_to_lum_u8`.

### Dead Code & Bug Fixes
- **Duplicate Guard Removal**: Removed duplicated `if !input.is_dir()` check in `metadata pack`.
- **Unused Variable Cleanup**: Dropped unused `mtimes`/`mt` tracking in `metadata watch` and unreferenced `what` in the pipeline and translate modules.
- **Test Regression & Clippy Fixes**: Fixed `stub_ocr_rec_is_rejected_before_model_load` in `tests/tauri_batch.rs` to target the unsupported `cht` script now that RapidOCR is fully implemented; corrected struct update syntax; ensured zero clippy warnings (`cargo clippy --all-targets --features tauri -- -D warnings`).

---

## [0.2.0] - 2026-09-11

**Major release**: Rapid OCR 5-language implementation + full editor backend + Tauri bindings.

### Added
- **Rapid OCR 5-Language Full Implementation**:
  - CTC decode with argmax → blank collapse, vertical text rotation
  - Multi-language: PP-OCRv3 models for JP (3.6MB), EN (9.0MB), KR (3.3MB), CN (10.7MB)
  - Auto-download from HuggingFace/PaddleOCR to `~/.cache/kzktdk/models/rapid/`
  - Session cache (`Arc<Mutex<Session>>`) per language, loaded once per process
  - Auto-detect: trial-decode EN→JP→KR→CN, picks best confidence (threshold 0.3)
  - Performance: 4.7s → 2s per page (57% faster), 5 pages in 10.4s
- **Editor Backend** (7 commands):
  - `metadata export`: Detect → JSON (no LLM)
  - `metadata edit`: 20+ flags (set/bbox/add/delete/style/stdin-patch transactional)
  - `metadata render`: Single + batch (--project + --jobs auto)
  - `metadata preview`: Live bubble preview (--to-stdout + --thumb)
  - `metadata show`: Table view (--id filter, --format json)
  - `metadata watch`: Auto re-render dirty pages (--once for CI)
  - `metadata pack`: Folder → CBZ
- **Font Management** (5 commands): import/list/remove/set-default/get-default with registry `~/.local/share/kzktdk/fonts`
- **Tauri Bindings** (`src/tauri.rs`): 7 GUI-ready functions (load/apply_patch/render/preview/detect/pack/batch_translate)
- **Translation Features**: cache, glossary, retry-failed, progress JSONL, reading order, mask brush, PDF support
- **CLI Modularization**: `src/main.rs` 3743→36 lines, split to `src/cli/*`
- **Supply Chain**: audit.yml workflow, EUPL-1.2 inpaint accepted, MIT OR Apache-2.0 dual license

### Changed
- `TranslationContext.ocr_script` now enum (was String)
- CLI: `--ocr-script` accepts `auto|jp|en|kr|cn|cht`
- Help text: rapid OCR fully documented, manga deprecated
- Constants centralized to `src/config.rs`

### Removed
- Manga OCR implementation (−50 lines), `--ocr manga` shows deprecation warning

### Fixed
- Session cache lifetime: extract tensor inside lock
- Zero `unwrap`/`expect` on runtime paths
- Help text updated for 5-lang support

### Documentation
- README: +68 lines (editor backend, font management, OCR options)
- DOCUMENTATION.md: OCR §8 rewritten (5-lang architecture, performance)
- CHANGELOG: Full dev.1-18 history documented

### Tests
- 20 unit + 14 CLI contract + 9 Tauri tests, all green
- Live verification: JP manga 5 bubbles in 1.7-2s

---

Branch: `dev-kz-debug` (merged from `feat/rapid-rec-auto`)

### Added
- **Rapid OCR 5-Language Full Implementation**:
  - **CTC Decode** (`6a22307`): `rapid_recognize_crop()` with argmax → CTC collapse (blank/duplicates), vertical text rotation (h>w), normalize [0,1], resize height=32, return (text, confidence).
  - **Multi-Language** (`0e6f5d7`, `5f1d70c`): PP-OCRv3 models for JP (3.6MB), EN (9.0MB), KR (3.3MB), CN (10.7MB) via `config::rapid_model_urls()`. Auto-download from HuggingFace/PaddleOCR to `~/.cache/kzktdk/models/rapid/`. Traditional Chinese (cht) returns clear "not yet supported" error (no ONNX available).
  - **Session Cache** (`bfda021`): `Arc<Mutex<Session>>` via `OnceLock` per language, loaded once per process. Extract tensor data inside lock (lifetime fix). Performance: 4.7s → 2s per page (57% faster), 5 pages in 10.4s (~2s/page).
  - **Auto-Detect** (`bfda021`): `trial_decode_language()` tests EN→JP→KR→CN with 2-3 sample boxes, picks best confidence (threshold 0.3). Integrated in `rapid_detect_ort` for `OcrScript::Auto`, logs `[rapid-auto] detected Japanese (conf 0.87)`.
  - `TranslationContext.ocr_script` now enum (was String), updated 16 call sites (cli/pipeline/tauri/tests).

### Removed
- **Manga OCR Cleanup** (`baa85e5`): Deleted `MangaOcr` struct (−50 lines), `engine_rec_stub()` returns false (no stubs remain), `--ocr manga` shows deprecation warning → noop fallback.

### Changed
- CLI: `--ocr-script` now accepts `auto|jp|en|kr|cn|cht` (was just `jp`).
- Tests: 20 unit + 14 CLI green, rapid no longer stub, cht unsupported error tested.

### Fixed
- Session cache lifetime issue: extract tensor data inside lock before releasing (was: return reference to dropped session).
- Live verification: JP manga 5 bubbles OCR→translate in 1.7-2s (down from 4.7s), batch 5 pages 10.4s.

---

## [0.1.1-dev.17] - 2026-09-10

Branch: `refactor/de-smell` (technical debt audit results, 4 phases, 4 commits)

### Changed (Phase 1 — god file split)
- `src/main.rs` 3,743 lines → 36 lines (only `Cli` parse + dispatch).
  Each match arm moved verbatim (zero behavioral diff) to `src/cli/`:
  `args.rs` (CLI definitions), `detect.rs`, `translate.rs` (+ `CANCELLED`,
  `PageRecord`, `retry_hit`, `translate_summary`, `finish_translate`),
  `metadata.rs`, `font.rs`, `decrypt.rs`, `inpaint.rs`, `util.rs`
  (helpers + `print_developer_help`). `draw_rect` moved to `detect.rs`
  (sole consumer). Verification: `--help` byte-identical, all tests green.
- Mechanical fixes from split: `include_bytes!("../fonts/…")` →
  `../../fonts/…`, match closing delimiters fixed in metadata/font.

### Changed (Phase 2 — constants centralized to new `src/config.rs`)
- Single source for: LLM endpoints (`OPENAI_DEFAULT_BASE_URL`,
  `GEMINI_API_BASE`, `ANTHROPIC_API_URL/VERSION`,
  `gemini_generate_url()` — tested byte-identical), CLI defaults
  (model, font, target, rate, batch, jobs, watch interval, inpaint output),
  LLM tuning (`LLM_TEMPERATURE`, `CLAUDE_MAX_TOKENS`), retry/backoff
  (`RATE_LIMIT_RETRY_MAX`, `RETRY_BACKOFF_BASE_MS/MAX_SHIFT` — 1s/2s/4s/8s
  schedule tested), vision (`IMAGE_U8_DIVISOR` bit-identical,
  `MERGE_NEARBY_GAP_PX`), metadata lock-file, OCR download URLs.
- `default_value` literals in clap → `default_value_t`/`default_value`
  referencing consts (`--help` remains identical, verified by diff).
- `tauri::TranslateConfig` INTENTIONALLY keeps its own GUI defaults
  (Gemini/Claude models + batch_size differ from CLI); only OpenAI URL
  is shared. YOLO 6.0/0.8 values were already constants
  (`FALSE_GIANT_*`) — left unchanged.
- `rg https?:// src/` now matches only `config.rs` + help-text.

### Fixed (Phase 3 — eliminated `unwrap`/`expect` on runtime paths)
- ZERO `unwrap`/`expect` in `src/cli/`, `editor.rs`, `translation/`,
  `pipeline.rs`, `cache.rs` on runtime paths (remaining only in `#[cfg(test)]`
  + 2 `// OK:` comments on impossible invariants + `unwrap_or*`).
- `cache.rs`: `Mutex::lock` poisoning → `anyhow` error (no panic).
- `cli::translate`: unknown provider + pool-YOLO exhausted → graceful
  per-page failure (original copied, marked `failed` in summary) without panic;
  `finish_translate` now `-> Result`; semaphore/pool uses
  match-`Err` → failure tuple; `file_name()` via helper
  `util::file_name()` (`with_context`); JSON serialization uses
  `.context()?`.
- `cli::detect`: closure `make_page_with_ft` now `-> Result` (+ `?` at
  2 call-sites); `cli::metadata` sort closure uses `unwrap_or_default`
  (not a `Result` path); `Mask --from` via let-else `bail!`.
- New test `tauri_batch::hostile_inputs_fail_gracefully_never_panic`:
  corrupt/0-byte images → per-page `failed`, directory-as-input and
  file-as-out-dir → setup `Err` — without panicking.

### Changed (Phase 4 — OCR rec explicitly marked UNSUPPORTED, option b)
- Decision: Full `rec` implementation DEFERRED; stub made honest + fail-fast
  (rationale: previous stub returned fake Japanese text `テスト`/`フリーテキスト`/`テキスト`
  silently polluting `--mode ocr` and `raw_text`).
- `src/ocr.rs`: New `engine_rec_stub()` + `ensure_rec_available()`;
  `RapidOcr`/`MangaOcr::recognize` → `None` + one-time warning;
  `recognize_regions` → empty/honest (real detection boxes preserved
  without placeholder); `tesseract` UNCHANGED (real via external binary).
- Fail-fast before model loading (hermetic, no model/network):
  `translate_page`, `cli::detect::run`, `cli::translate::run`,
  `tauri::editor_translate_batch`. `--mode auto` still falls back to vision.
- `--ocr --help` + new `DOCUMENTATION.md §8` document status.
- Tests: 3 unit `ocr::tests`, 1 hermetic CLI contract, 1 hermetic Tauri.
- Total tests: 19 unit + 14 CLI contracts + 9 Tauri, all green;
  `clippy --all-targets` 0 errors with/without feature.

---

## [0.1.1-dev.16] - 2026-09-10

Branch: `dev-kz-debug`

### Added
- **YOLO `Send` verified + model pool**: `send_tests` (`yolo.rs`, `editor.rs`)
  proves `ort::Session`/`YoloModel`/`EditorSession: Send` at compile time.
  Parallel `translate` path loads **one model per worker** (`pool_size =
  min(jobs, pages)`, checkout pop/push via `std::Mutex`, never held across
  awaits) instead of loading per page. Measured (release):
  `YoloModel::new` ~205–215ms steady (~340ms initial load); 20-page chapter
  with 8 workers saves ±(20−8)×0.21s ≈ 2.5s + avoids memory spikes.
  Verified live: 4 pages `jobs=2` without deadlocks, `done` summary correct.
- **Pipeline moved to library** (`src/pipeline.rs`): `TranslationContext` +
  `build_provider` + `translate_page` extracted from CLI binary for GUI
  reusability; CLI behavior identical (`events: None`). `TranslationContext`
  adds optional `events: Option<&dyn Fn(PageEvent) + Send + Sync>`; when set,
  all terminal output is suppressed and phases
  (`detect_end/translate_end/render_end`) are routed via callback.
  Shared event type: `pipeline::PageEvent { idx, total, page, phase, error }`.
- **Tauri batch translate**: `tauri::{TranslateConfig (..Default..),
  ProgressEvent (= PageEvent), PageResult, editor_translate_batch}` —
  sequential, single YOLO model, internal current-thread runtime, no
  `println!`/exit. Setup validation (images/provider/glossary/model/font)
  returns `Err` before page execution; per-page failure copies original
  (CLI parity) + terminal event `done`/`failed`. Note: callback requires
  `Send + Sync` (not just `Send`) because it is shared via
  `TranslationContext.events` which must be `Send` for CLI parallel path.
- **PDF**: `scripts/verify_pdf.sh` (library resolution + `pdf_roundtrip_or_skip`,
  `PDF_OK`/`PDF_SKIP` banner, no LLM/network) + `docs/PDFIUM.md` (OS table,
  `bblanchon/pdfium-binaries` build source, `PDFIUM_LIB_PATH` setup,
  verification). On this machine: `PDF_SKIP` + PDF command fails gracefully
  (exit 1 + message, verified).
- **Docs**: `README.md` (`translate` options table: `--glossary/--progress/`
  `--format/--quiet/--retry-failed/--export pdf`, PDF + JSONL +
  `--format json` examples, Exit Codes table, links to `§7`/`PDFIUM.md`/`GPU_ROADMAP.md`),
  `DOCUMENTATION.md §6` links `docs/PDFIUM.md` + verification script.
- **Research (no implementation)**: `docs/GPU_ROADMAP.md` (cargo features +
  `with_execution_providers` for `ort` 2.0: CUDA/TensorRT/DirectML/CoreML,
  OS dependencies, CPU fallback mandatory) + `docs/UPDATE_ROADMAP.md`
  (`tauri-plugin-updater` v2: signing, artifacts, `latest.json` via
  `tauri-action`, preserve artifact naming).
- **Tests**: `tests/tauri_batch.rs` (7 tests, gated by `--features tauri`;
  0 tests without feature): defaults, empty/missing-image/unknown-provider/
  glossary-bad/model-missing rejected, plus offline e2e (local model +
  closed-port LLM endpoint) `detect_end` → `failed` + original copied.
  `tests/cli_contract.rs` +2 hermetic regressions (without key): `translate
  --export pdf` produces file (not directory), and stdout
  `--format json --quiet` remains pure JSON on cache hit.
  Total: 14 unit + 13 CLI contracts + 7 Tauri, green; `clippy --all-targets`
  0 errors with/without feature.

### Fixed (from live Gemini + PDF testing)
- **PDF double-bind**: `bind_pdfium()` (`src/archive.rs`) failed with
  `PdfiumLibraryBindingsAlreadyInitialized` on second PDF usage in same
  process (PDF input + `--export pdf`, round-trip test). Handle now cached
  once per process (`once_cell::sync::OnceCell<Pdfium: Send+Sync>`, new
  `once_cell` dep — already in dependency tree). `pdf_roundtrip_or_skip`
  genuinely passes (`PDF_OK`) with `bblanchon/pdfium-binaries`.
- **`--export pdf -o file.pdf` created directory `file.pdf`**
  (`create_dir_all(temp_output_dir)`), causing packing failure with
  `IsADirectory`. PDF archives now use tempdir like CBZ; `-o` only names
  the final file.
- **Cache-hit polluted stdout**: `[Cache Hit]` used unconditional `println!`,
  causing stdout with `--format json` to not be pure JSON. Now routed through
  `tinfo!` (stderr on `--quiet`), and `tinfo!` is fully silenced in GUI event
  mode. Verified: re-running PDF served from cache, stdout remains pure.
- Live test: ZIP 5/5 pages + PDF 5 pages → Indonesian via
  `gemini-3.1-flash-lite`, exit 0, valid 5-page PDF.

### Changed
- `Cargo.toml` version kept at `0.1.0` (repo practice: dev tagged in changelog).
- Stale comment "create per task via model_file clone" updated.
- `tauri.rs`: font helper `load_font_bytes()` shared between `session_for()` +
  batch functions.

---

## [0.1.1-dev.15] - 2026-09-10

Branch: `dev-kz-debug`

### Added
- **Translate progress & robustness**:
  - `translate --progress text|jsonl --quiet --format text|json --retry-failed <DIR> --glossary <dictionary.json>` — JSONL per phase (`detect_end/translate_end/render_end`) + final summary (`files/ok/fail/skipped/glossary_hits/misses/output`) to stdout if `--format json`; exit 1 if any page fails.
  - Ctrl-C: completes running page, cancels queue (`cancelled`), exit 130.
  - `--retry-failed`: reuses previous successful output (file exists + sidecar contains translation), only failed pages are re-translated.
  - `TranslationContext` holds `glossary/progress/quiet/page_idx/total` + atomic counter; `translate_with_chain/repair/RateLimiter` respect quiet mode (log to stderr).
- **Glossary**: `load_glossary()` (validates 500 entries, case-insensitive deduplication, exit 2) injected as `GLOSSARY (must obey)` block + `enforce_glossary()` rewrites once per leaked term.
- **Reading order**: `preparer::detect_reading_order(R2L|L2R)` (column + bridging titles) + `export --reading-order r2l|l2r|off` (default `r2l`); `translate --save-metadata` populates `order`.
- **Mask brush**: `inpaint::{MaskedRegion,load_mask_for,inpaint_regions,inpaint_crop_with_mask}`; `render_page/preview_bubble` uses `mask_path` (fails fast on dimension mismatch); `metadata mask --id --from/--clear` (exit 2 on invalid).
- **Tauri binding**: `EditorSession::open_model/detect_bubbles/export_page`, `png_base64()`, module `src/tauri.rs` (feature `tauri`) contains `Result<T,String>` wrappers ready for `#[tauri::command]`.
- **PDF**: `archive::{is_pdf_path,extract_pdf_pages,create_pdf,translated_pdf_name}` via `pdfium-render` (binds `PDFIUM_LIB_PATH` → system lib, clear error if missing); `translate --export pdf` + auto if input/-o is `.pdf`; `detect/export` accepts PDF.
- **Machine-readable**: `detect --format json [--quiet]`, `pack --format json`; exit code matrix `0/2/1/130` documented.
- **Tests & docs**: `tests/cli_contract.rs` (11 tests: JSON stdout, exit codes, stream cleanliness, transactionality, mask, pack, help flags, PDF graceful) + `DOCUMENTATION.md §6 PDF` & `§7 GUI Contract`.

---

## [0.1.1-dev.14] - 2026-09-10

Branch: `dev-kz-debug`

### Added
- **P0 machine-readable backend (GUI)**:
  - `metadata show/validate --format text|json [--quiet]`, `font list/get-default --format text|json [--quiet]` — JSON to stdout, logs to stderr; `validate --format json` exit 2 when invalid (`ValidationReport{valid,kind,errors,meta}`).
  - `metadata preview --to-stdout --thumb 512` — single-line PNG base64 to stdout (no disk), logs to stderr, downscales longest side.
  - `metadata render --progress text|jsonl` — JSONL per page + `{"done":true}` to stderr (single & batch).
- **P1 editor facade**:
  - New `src/editor.rs`: `EditPatch` (all edit flags + `order` + `mask_path`), `apply_patch()` (lenient & strict), `EditorSession::{new,load_page,render_page,preview_bubble}`, `thumbnail()`, `bubbles_hash()`.
  - `metadata edit --stdin-patch` — transactional JSON patch from stdin, aborts without writing + exit 2 on error.
  - Sibling `.lock` file lock (`create_new` + retry 50x10ms) in `atomic_write_json` — safe for concurrent GUI-vs-CLI operations (tested).
  - `metadata render` single/batch + `preview` now use `EditorSession` (removes ~120 lines of duplicate code).
- **P2 watch & schema**:
  - `metadata watch --project --images -o [--interval 1] [--once]` — incremental poll, only re-renders dirty pages (`bubbles` hash), JSONL to stderr.
  - Schema: `Bubble.mask_path?`, `PageEditData.order?` (+ `ordered_bubbles()`, `validate_page()` checks `unknown_order_id`), `metadata edit --order "1,ft1,2" --mask-path "ID=path"`.

---

## [0.1.1-dev.13] - 2026-09-10

Commit: `f2a4ca3` — `feat: freetext outside bubbles + rapid ort real (vision/ft, auto-download, Indo proof)`

Branch: `dev-kz-debug`

### Added
- **Freetext Outside Bubbles (kzkt parity)**:
  - `src/preparer.rs` `detect_free_text()` `WHITE` mask `scale 2048` `canvas.drawRect` bubble → `OcrEngine.recognize_regions` filter `w<12||h<8` `len==1 alnum` `center insideBubble` `IoU>=0.3` `mergeNearby gap20` `freetext_pad 6%` `bg median`.
  - `src/ocr.rs` `TextRegion{bbox,text}` `OcrScript Japanese/English/Korean/Chinese/Auto` `trait OcrEngine {recognize, recognize_regions}` `NoopOcr` `RapidOcr {det_path,rec_path,dict_path}` `MangaOcr` `TesseractOcr {jpn_vert+jpn psm6 tsv conf>=30}` `create_ocr_engine()` `cache_dir_for ~/.cache/kzktdk/models/<engine>/` `ensure_rapid_cached()` auto-download `curl -L` `ch_PP-OCRv3_det_infer.onnx 2.4M + japan_rec_crnn.onnx 3.5M + japan_dict.txt 17K` `SWHL/RapidOCR` `~/.cache/kzktdk/models/rapid 5.8M` `ort 2.0.0-rc.13` `Session::builder().commit_from_file` letterbox `640` threshold `0.3` BFS real boxes `ft1 [374,277,616,378]` instead of dummy `[20,20]`.
  - CLI `translate/detect --ocr none|rapid|manga|tesseract|vision --translate-free-text --ocr-script jp --mode vision|ocr|auto --ocr-model <path>` `TranslationContext` `combined_dets ft1..` `crops` padded `raw_map` `vision mosaic` vs `ocr JSON` vs `auto` fallback `norm_map` lowercase `ft` `inpaint_targets combined` `Typesetter render_bubble_text_with_style` `metadata PageEditData ocr_engine ft conf0.90`.
  - `src/translation/mod.rs` prompt: `RED ID numeric or ft1/ft2 include ALL IDs even if SKIP` to prevent LLM from skipping freetext.

### Fixed
- **Rapid Dummy → Real**: `RapidOcr` previously returned dummy `[20,20] テスト` `SKIP`; now runs real `ort det`: `Sonico page01 ft1 [374,277,616,378] → Band beranggotakan tiga wanita...` with 91% pixel difference `ft1_render_proof` in `sonico01_indo.jpg`.
- **FT Uppercase**: `norm_map` lowercases `FT1→ft1` before inpaint/typeset to prevent missing `all_translations`.

### Changed
- `translate_page` `detect` single+batch `make_page_with_ft` `draw_rect green` `batch json` `project.kedit.json`.

---

## [0.1.1-dev.12] - 2026-09-10

Commit: `f9af774` — `feat: ocr prep — raw_text, bold/italic, batch style, backup`

Branch: `dev-kz-debug`

### Added
- **OCR Prep — Raw Text & Stub Engine**:
  - Added `Bubble.raw_text` and `PageEditData/Project.ocr_engine` (`Option<String>`) with `serde default` for backwards compatibility with legacy JSON.
  - Module `src/ocr.rs` `trait OcrEngine` + `NoopOcr` stub — pipeline remains Vision-only, `raw_text` stays `None` until an engine is selected.
  - Flags `translate --ocr <none|vision|local>` (dummy, fallback to none) and `metadata edit --raw-text "ID=text"` + `preview --show-raw` to inspect `raw_text` without saving.
  - `metadata show` columns `RAW` and `ocr` in header, unicode-safe `chars()` handling for Japanese text.

- **Bold / Italic & Batch Style**:
  - `BubbleStyle { is_bold, is_italic }` + flags `metadata edit --bold/--italic "ID=true"` and `--apply-style "align=center,bold=true,font_size=20"` across all bubbles.
  - `--replace "old=new"` find & replace across all `translated` with `edited=true`.

- **Backup Lightweight Undo**:
  - `atomic_write_json` saves `.kedit.json.bak.<ts>` before overwriting, keeping the 3 newest — replaces `kzkt` GUI undo without in-memory history.

### Changed
- `metadata export` for single page writes an object (not array) for `load_page_metadata` compatibility; multi-page remains an array.

### Fixed
- `metadata show` panic on unicode `raw_text` (byte slice `..10` → `chars().take(10)`).

---

## [0.1.1-dev.11] - 2026-09-10

Commit: `29823a9` — `feat: CLI editor final (pack/show/preview override/edit bg)`

Branch: `dev-kz-debug`

### Added
- **Metadata Show & Project List**:
  - `metadata show <json> [--id ID]` auto-detects `PageEditData` vs `project.kedit.json` — Page displays `ID | BBOX | CONF | EDIT | STYLE | TRANSLATED` table, Project displays `pages` list with bubble counts.
  - Supports `--id` filter to focus on a single bubble and validates `width/height` bounds in output.

- **Pack to CBZ**:
  - `metadata pack <folder> -o chapter.cbz` packs rendered folder into a CBZ using natural sorting (1, 2, 10) and `page_0001.jpg` naming.
  - Handles empty folders with a clear `No images found` message.

- **Batch Render from Project**:
  - `metadata render --project project.kedit.json --images <folder> -o rendered/ --jobs auto` renders all pages from `project.kedit.json` without LLM calls, with optional `--images` to override source image location and `--jobs` for parallelism.
  - Single mode `metadata render <image> --metadata page.kedit.json -o out.jpg` remains fully compatible.

- **Preview Style Overrides**:
  - `metadata preview --id 1 --text "Hello" --font-size 28 --text-color 255,0,0 --align center` allows testing styles without modifying JSON, ideal for live editor integration.
  - Supports overriding `font-family`, `font-size`, `text-color`, `stroke-color`, `align`.

- **Edit Enhancements**:
  - New flags `--bg-color "ID=R,G,B"` and `--conf "ID=0.99"` for background color correction and detector confidence tuning.
  - `--clear-style "ID"` resets all per-bubble styling in a single command.
  - `bbox` validation verifies `x2<=width && y2<=height` and `x1<x2`, preventing coordinates outside page bounds.

- **CLI Discoverability**:
  - `kzktdk -h` displays examples for `metadata` and `font`, listing subcommands `Commands: translate, metadata, font`.

### Changed
- Signatures for `metadata render` and `metadata preview` extended with optional flags while maintaining backwards compatibility.
- `metadata validate` auto-detects `Project` vs `PageEditData` and reports warnings for duplicate IDs or out-of-bounds bounding boxes with target language and prompt signature info.

### Fixed
- Help template previously only showed `translate`; now displays the full editor workflow (`export → edit → render/preview → show/pack`).

---

## [0.1.1-dev.10] - 2026-09-10

Commit: `853236e` — `fix: translate_page use with_style for consistency`

Branch: `dev-kz-debug`

### Fixed
- `translate_page` now uses `render_bubble_text_with_style` for consistency with `metadata render`, preparing per-bubble style support across the translation pipeline.

---

## [0.1.1-dev.9] - 2026-09-10

Commit: `c10a510` — `chore: update .gitignore and remove debug bins`

Branch: `dev-kz-debug`

### Changed
- Added `src/bin/test_*.rs` to `.gitignore` and removed temporary debug binaries `test_deser.rs`/`test_deser2.rs`.

---

## [0.1.1-dev.8] - 2026-09-10

Commit: `33b9ec6` — `docs: update changelog hash for eb68297`

Branch: `dev-kz-debug`

### Fixed
- Corrected commit hash `a9536cd → eb68297` in entry `0.1.1-dev.7` after amend.

---

## [0.1.1-dev.7] - 2026-09-10

Commit: `eb68297` — `fix: metadata render style override (font_size/color/align)`

Branch: `dev-kz-debug`

### Fixed
- `metadata render` previously only respected `font_family`, ignoring `font_size`, `text_color`, `stroke_color`, and `align`; now applies full `BubbleStyle` per bubble with a custom Typesetter.
- Removed temporary debug logging after verification.

---

## [0.1.1-dev.6] - 2026-09-10

Commit: `ff6225f` — `docs: changelog Stage 1+2 font & style`

Branch: `dev-kz-debug`

### Changed
- Added entries for `0.1.1-dev.4` (Stage 1 font registry) and `0.1.1-dev.5` (Stage 2 style editing) for per-commit tracking.

---

## [0.1.1-dev.5] - 2026-09-10

Commit: `093bc28` — `feat: bubble style editing + preview (Stage 2)`

Branch: `dev-kz-debug`

### Added
- **Bubble Style Editing & Preview**:
  - Added `BubbleStyle` (`font_size`, `text_color`, `stroke_color`, `align`) and `edited` flag for manual tracking.
  - `metadata edit --font-size/--text-color/--stroke-color/--align/--edited` to modify style per bubble.
  - `metadata preview <image> --metadata --id --text -o preview.jpg` renders single bubble in <100ms without saving JSON — ready for live editor.
  - `Typesetter` supports `font_size` override (min/max clamping), text/stroke colors, and `left/center/right` alignment.

---

## [0.1.1-dev.4] - 2026-09-10

Commit: `a3671f2` — `feat: font registry global+per-bubble (Stage 1)`

Branch: `dev-kz-debug`

### Added
- **Global & Per-Bubble Font Registry**:
  - `FontRegistry` (`import/list/remove/resolve`) manages pool `~/.local/share/kzktdk/fonts` and `AppConfig` (`~/.config/kzktdk/config.toml`) for Latin/CJK defaults.
  - `font` subcommand: `import`, `list`, `remove`, `set-default`, `get-default` with `FontArc` validation.
  - `translate --font <name>` accepts registry font names in addition to file paths.
  - `metadata edit --font-family` and `metadata render` support per-bubble font override via custom Typesetter.

---

## [0.1.1-dev.3] - 2026-09-10

Commit: `a7f7c4e` — `feat: metadata add/delete box`

Branch: `dev-kz-debug`

### Added
- **Bubble Add/Delete**:
  - `metadata edit --add "ID=x1,y1,x2,y2[=text]"` adds new bubble with duplicate validation and `x1<x2 && y1<y2`.
  - `metadata edit --delete "ID"` deletes bubble by ID.
  - Supports text containing `=` and provides `Added/Deleted/already exists/not found` logging.

---

## [0.1.1-dev.2] - 2026-09-10

Commit: `7c850b2` — `feat: CLI editor-ready metadata separation`

Branch: `dev-kz-debug`

### Added
- **Editor-Ready Metadata Separation**:
  - `PageEditData`/`Project` (`version`, `page`, `width/height`, `target_lang`, `prompt_sig`, `bubbles`) with `atomic_write_json` and `save/load_page_metadata`/`save_project`.
  - `translate --save-metadata` saves sidecar `.kedit.json` per page + `project.kedit.json` (single & batch `JoinSet` parallel).
  - `metadata` subcommand: `export` (detect without LLM), `render` (inpaint+typeset without LLM), `edit --set/--bbox`, `validate`.
  - `detect` batch `folder/cbz/epub` via `prepare_input`, `--json`, `--jobs auto` with natural sorting.

---

## [0.1.1-dev.1] - 2026-09-09

Commit: `c35ad0d` — `feat: port cache, inpaint parallel, batch parallel, fallback, rate-limit, repair, epub`

Branch: `dev-kz-debug` (from `master ceda763`)

### Added
- **Translation Cache & Performance**:
  - `TranslationCache` (`rusqlite` + `blake3`, `prompt_signature` custom/classic) at `~/.cache/kzktdk/cache.db` with `--no-cache`/`--clear-cache`.
  - `ProviderChain` fallback (`--fallback-provider`) and `RateLimiter` (`--rate-limit 3`, backoff 1s/2s/4s) with `repair_json_output`.
  - `inpaint_image` parallelized via `rayon` and `translate` batch parallelized via `JoinSet + Semaphore(jobs=auto)`.
  - `EPUB` archive support as ZIP skipping `META-INF` with natural sorting.

---

## [0.1.1] - 2026-09-10

Commit: `c762821` — `chore: update Cargo.lock`

Branch: `dev-kz-debug`

### Changed
- Updated `Cargo.lock` for `serde_derive`.

---

## [0.1.0] - 2026-09-09

Commit: `ceda763` — `feat: initial release of kzktdk`

### Added
- Initial Rust port of `kouzen-neo/kzkt` Android — `YoloModel` cascade 3-stage, Rust Telea `inpaint`, `MosaicBuilder`, elliptic `Typesetter`, natural sort `archive`.
- CLI: `translate`, `detect`/`inpaint`/`decrypt-model` (hidden) with `fonts/Komika Axis` & `KosugiMaru`.

### Docs
- `README.md`, `DOCUMENTATION.md`, `CONTRIBUTING.md`.
