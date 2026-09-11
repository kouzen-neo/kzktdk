# Changelog

Semua perubahan penting pada proyek ini didokumentasikan di file ini.

Format berdasarkan [Keep a Changelog](https://keepachangelog.com/id/1.0.0/),
dan proyek ini mengikuti [Semantic Versioning](https://semver.org/lang/id/).

## [Unreleased]

### Rencana
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

Branch: `refactor/de-smell` (hasil audit hutang teknis, 4 fase, 4 commit)

### Changed (Fase 1 — god file dipecah)
- `src/main.rs` 3743 baris → 36 baris (hanya `Cli` parse + dispatch).
  Tiap match arm pindah utuh (verbatim, nol diff perilaku) ke `src/cli/`:
  `args.rs` (definisi CLI), `detect.rs`, `translate.rs` (+ `CANCELLED`,
  `PageRecord`, `retry_hit`, `translate_summary`, `finish_translate`),
  `metadata.rs`, `font.rs`, `decrypt.rs`, `inpaint.rs`, `util.rs`
  (helper + `print_developer_help`). `draw_rect` ikut `detect.rs`
  (satu-satunya pemakai). Verifikasi: `--help` byte-identical, semua test hijau.
- Perbaikan mekanis akibat pindah: `include_bytes!("../fonts/…")` →
  `../../fonts/…`, penutup `match` yang ikut terbuang di metadata/font.

### Changed (Fase 2 — konstanta disentralisasi ke `src/config.rs` baru)
- Satu sumber untuk: endpoint LLM (`OPENAI_DEFAULT_BASE_URL`,
  `GEMINI_API_BASE`, `ANTHROPIC_API_URL/VERSION`,
  `gemini_generate_url()` — diuji byte-identical), default CLI
  (model, font, target, rate, batch, jobs, watch interval, inpaint output),
  tuning LLM (`LLM_TEMPERATURE`, `CLAUDE_MAX_TOKENS`), retry/backoff
  (`RATE_LIMIT_RETRY_MAX`, `RETRY_BACKOFF_BASE_MS/MAX_SHIFT` — jadwal
  1s/2s/4s/8s diuji), vision (`IMAGE_U8_DIVISOR` bit-identical,
  `MERGE_NEARBY_GAP_PX`), lock-file metadata, URL unduhan OCR.
- `default_value` literal di clap → `default_value_t`/`default_value`
  merujuk const (`--help` tetap identik, terverifikasi diff).
- `tauri::TranslateConfig` SENGAJA menyimpan default GUI-nya sendiri
  (model Gemini/Claude + batch_size memang beda dari CLI); hanya URL
  OpenAI yang dishare. Nilai 6.0/0.8 YOLO ternyata sudah const
  (`FALSE_GIANT_*`) — tidak diubah.
- `rg https?:// src/` kini hanya mengenai `config.rs` + help-text.

### Fixed (Fase 3 — steril `unwrap`/`expect` di path runtime)
- NOL `unwrap`/`expect` di `src/cli/`, `editor.rs`, `translation/`,
  `pipeline.rs`, `cache.rs` pada path runtime (sisa hanya di `#[cfg(test)]`
  + 2 komentar `// OK:` pada invarian yang mustahil + `unwrap_or*`).
- `cache.rs`: `Mutex::lock` poison → error `anyhow` (bukan panic).
- `cli::translate`: provider tak dikenal + pool-YOLO exhausted → gagal
  per-halaman graceful (original disalin, `failed` di summary) bukan panic;
  `finish_translate` kini `-> Result`; semaphore/pool memakai
  match-`Err` → failure tuple; `file_name()` lewat helper
  `util::file_name()` (`with_context`); serialisasi JSON memakai
  `.context()?`.
- `cli::detect`: closure `make_page_with_ft` kini `-> Result` (+ `?` di
  2 call-site); `cli::metadata` sort closure memakai `unwrap_or_default`
  (bukan path `Result`); `Mask --from` via let-else `bail!`.
- Test baru `tauri_batch::hostile_inputs_fail_gracefully_never_panic`:
  gambar korup/0-byte → `failed` per halaman, direktori-jadi-input dan
  file-jadi-out-dir → `Err` setup — tanpa panic.

### Changed (Fase 4 — OCR rec dinyatakan UNSUPPORTED eksplisit, opsi b)
- Keputusan: implementasi `rec` beneran DITUNDA; sebagai gantinya stub
  dibuat jujur + fail-fast (alasan: stub lama mengembalikan teks Jepang
  palsu `テスト`/`フリーテキスト`/`テキスト` yang diam-diam meracuni
  `--mode ocr` dan `raw_text`).
- `src/ocr.rs`: `engine_rec_stub()` + `ensure_rec_available()` baru;
  `RapidOcr`/`MangaOcr::recognize` → `None` + warning sekali;
  `recognize_regions` → kosong/jujur (kotak deteksi real dipertahankan,
  tanpa placeholder); `tesseract` TIDAK diubah (real via binary eksternal).
- Fail-fast sebelum load model (hermetik, tanpa model/jaringan):
  `translate_page`, `cli::detect::run`, `cli::translate::run`,
  `tauri::editor_translate_batch`. `--mode auto` tetap fallback vision.
- `--ocr --help` + `DOCUMENTATION.md §8` baru mendokumentasikan status.
- Test: 3 unit `ocr::tests`, 1 kontrak CLI hermetik, 1 Tauri hermetik.
- Total test: 19 unit + 14 kontrak CLI + 9 tauri, hijau;
  `clippy --all-targets` 0 error dengan/tanpa feature.

---

## [0.1.1-dev.16] - 2026-09-10

Branch: `dev-kz-debug`

### Added
- **YOLO `Send` terbukti + model pool**: `send_tests` (`yolo.rs`, `editor.rs`)
  membuktikan `ort::Session`/`YoloModel`/`EditorSession: Send` saat kompilasi.
  Path paralel `translate` memuat **satu model per worker** (`pool_size =
  min(jobs, pages)`, checkout pop/push via `std::Mutex`, tak pernah ditahan
  lintas-await) alih-alih satu load per halaman. Terukur (release):
  `YoloModel::new` ~205–215ms steady (~340ms load pertama); chapter 20
  halaman/8 worker menghemat ±(20−8)×0,21s ≈ 2,5s + lonjakan memori.
  Diverifikasi live: 4 halaman `jobs=2` tanpa deadlock, ringkasan `done` benar.
- **Pipeline pindah ke lib** (`src/pipeline.rs`): `TranslationContext` +
  `build_provider` + `translate_page` keluar dari biner CLI agar bisa dipakai
  ulang GUI; perilaku CLI identik (`events: None`). `TranslationContext`
  mendapat field opsional `events: Option<&dyn Fn(PageEvent) + Send + Sync>`;
  bila di-set, semua output terminal ditekan dan fase
  (`detect_end/translate_end/render_end`) disalurkan via callback.
  Tipe event bersama: `pipeline::PageEvent { idx, total, page, phase, error }`.
- **Tauri batch translate**: `tauri::{TranslateConfig (..Default..),
  ProgressEvent (= PageEvent), PageResult, editor_translate_batch}` —
  sekuensial, satu model YOLO, runtime current-thread internal, tanpa
  `println!`/exit. Validasi setup (gambar/provider/glossary/model/font)
  mengembalikan `Err` sebelum halaman berjalan; kegagalan per halaman
  menyalin original (paritas CLI) + event terminal `done`/`failed`.
  Catatan: callback butuh `Send + Sync` (bukan hanya `Send`) karena dipakai
  bersama lewat `TranslationContext.events` yang harus `Send` untuk path
  paralel CLI.
- **PDF**: `scripts/verify_pdf.sh` (resolusi lib + `pdf_roundtrip_or_skip`,
  banner `PDF_OK`/`PDF_SKIP`, tanpa LLM/jaringan) + `docs/PDFIUM.md` (tabel
  per OS, sumber build `bblanchon/pdfium-binaries`, setup `PDFIUM_LIB_PATH`,
  verifikasi). Di mesin ini: `PDF_SKIP` + perintah PDF gagal graceful
  (exit 1 + pesan, terverifikasi).
- **Docs**: `README.md` (tabel opsi `translate`: `--glossary/--progress/`
  `--format/--quiet/--retry-failed/--export pdf`, contoh PDF + JSONL +
  `--format json`, tabel Exit Codes, tautan `§7`/`PDFIUM.md`/`GPU_ROADMAP.md`),
  `DOCUMENTATION.md §6` menautkan `docs/PDFIUM.md` + skrip verifikasi.
- **Riset (tanpa implementasi)**: `docs/GPU_ROADMAP.md` (cargo feature +
  `with_execution_providers` per `ort` 2.0: CUDA/TensorRT/DirectML/CoreML,
  dependensi per OS, fallback CPU wajib) + `docs/UPDATE_ROADMAP.md`
  (`tauri-plugin-updater` v2: signing, artefak, `latest.json` via
  `tauri-action`, penamaan artefak jangan diubah).
- **Tests**: `tests/tauri_batch.rs` (7 test, gate `--features tauri`;
  tanpa feature jadi 0 test): defaults, empty/missing-image/unknown-provider/
  glossary-bad/model-missing ditolak, plus e2e offline (model lokal +
  endpoint LLM port-tertutup) `detect_end` → `failed` + original tersalin.
  `tests/cli_contract.rs` +2 regression hermetik (tanpa key): `translate
  --export pdf` menghasilkan file (bukan direktori), dan stdout
  `--format json --quiet` tetap JSON murni saat cache hit.
  Total: 14 unit + 13 kontrak CLI + 7 tauri, hijau; `clippy --all-targets`
  0 error dengan/tanpa feature.

### Fixed (hasil uji live Gemini + PDF)
- **PDF double-bind**: `bind_pdfium()` (`src/archive.rs`) gagal dengan
  `PdfiumLibraryBindingsAlreadyInitialized` pada penggunaan PDF kedua dalam
  satu proses (input PDF + `--export pdf`, round-trip test). Kini handle
  di-cache sekali per proses (`once_cell::sync::OnceCell<Pdfium: Send+Sync>`,
  dep `once_cell` baru — sudah ada di tree). `pdf_roundtrip_or_skip` lolos
  nyata (`PDF_OK`) dengan libpdfium `bblanchon/pdfium-binaries`.
- **`--export pdf -o file.pdf` membuat direktori** `file.pdf`
  (`create_dir_all(temp_output_dir)`), lalu packing gagal `IsADirectory`.
  Kini arsip PDF memakai tempdir seperti CBZ; `-o` hanya menamai file akhir.
- **Cache-hit mencemari stdout**: `[Cache Hit]` memakai `println!`
  unconditional sehingga stdout `--format json` bukan JSON murni. Kini lewat
  `tinfo!` (stderr saat `--quiet`), dan `tinfo!` bungkam total dalam mode
  event GUI. Diverifikasi: run ulang PDF tersaji dari cache, stdout murni.
- Uji live: ZIP 5/5 halaman + PDF 5 halaman → Indonesia via
  `gemini-3.1-flash-lite`, exit 0, PDF 5 halaman valid.

### Changed
- `Cargo.toml` versi tetap `0.1.0` (praktik repo: dev ditandai changelog).
- Komentar basi "create per task via model_file clone" diperbarui.
- `tauri.rs`: helper font `load_font_bytes()` dipakai bersama
  `session_for()` + fungsi batch.

---

## [0.1.1-dev.15] - 2026-09-10

Branch: `dev-kz-debug`

### Added
- **Translate progress & robustness**:
  - `translate --progress text|jsonl --quiet --format text|json --retry-failed <DIR> --glossary <kamus.json>` — JSONL per fase (`detect_end/translate_end/render_end`) + ringkasan akhir (`files/ok/fail/skipped/glossary_hits/misses/output`) ke stdout bila `--format json`; exit 1 bila ada halaman gagal.
  - Ctrl-C: selesaikan halaman berjalan, batalkan antrean (`cancelled`), exit 130.
  - `--retry-failed`: pakai ulang output lama yang bagus (file ada + sidecar berisi terjemahan), hanya halaman gagal yang diterjemahkan ulang.
  - `TranslationContext` membawa `glossary/progress/quiet/page_idx/total` + counter atomik; `translate_with_chain/repair/RateLimiter` hormati mode senyap (log ke stderr).
- **Glossary**: `load_glossary()` (validasi 500 entri, anti-duplikat case-insensitive, exit 2) disuntik sebagai blok `GLOSSARY (must obey)` + `enforce_glossary()` rewrite sekali per istilah bocor.
- **Reading order**: `preparer::detect_reading_order(R2L|L2R)` (kolom + judul jembatan) + `export --reading-order r2l|l2r|off` (default `r2l`); `translate --save-metadata` ikut mengisi `order`.
- **Mask brush**: `inpaint::{MaskedRegion,load_mask_for,inpaint_regions,inpaint_crop_with_mask}`; `render_page/preview_bubble` memakai `mask_path` (gagal cepat bila ukuran salah); `metadata mask --id --from/--clear` (exit 2 bila invalid).
- **Tauri binding**: `EditorSession::open_model/detect_bubbles/export_page`, `png_base64()`, modul `src/tauri.rs` (feature `tauri`) berisi wrapper `Result<T,String>` siap-`#[tauri::command]`.
- **PDF**: `archive::{is_pdf_path,extract_pdf_pages,create_pdf,translated_pdf_name}` via `pdfium-render` (binding `PDFIUM_LIB_PATH` → system lib, error jelas bila absen); `translate --export pdf` + auto bila input/-o `.pdf`; `detect/export` terima PDF.
- **Machine-readable**: `detect --format json [--quiet]`, `pack --format json`; matriks exit code `0/2/1/130` didokumentasikan.
- **Tests & docs**: `tests/cli_contract.rs` (11 test: JSON stdout, exit code, kebersihan stream, transaksionalitas, mask, pack, help flags, PDF graceful) + bab `DOCUMENTATION.md §6 PDF` & `§7 GUI Contract`.

---

## [0.1.1-dev.14] - 2026-09-10

Branch: `dev-kz-debug`

### Added
- **P0 machine-readable backend (GUI)**:
  - `metadata show/validate --format text|json [--quiet]`, `font list/get-default --format text|json [--quiet]` — JSON ke stdout, log ke stderr; `validate --format json` exit 2 saat invalid (`ValidationReport{valid,kind,errors,meta}`).
  - `metadata preview --to-stdout --thumb 512` — PNG base64 satu baris ke stdout (tanpa disk), log ke stderr, downscale longest-side.
  - `metadata render --progress text|jsonl` — JSONL per halaman + `{"done":true}` ke stderr (single & batch).
- **P1 editor facade**:
  - `src/editor.rs` baru: `EditPatch` (semua flag edit + `order` + `mask_path`), `apply_patch()` (lenient & strict), `EditorSession::{new,load_page,render_page,preview_bubble}`, `thumbnail()`, `bubbles_hash()`.
  - `metadata edit --stdin-patch` — patch JSON transaksional dari stdin, abort tanpa tulis + exit 2 saat ada error.
  - File-lock sibling `.lock` (`create_new` + retry 50x10ms) di `atomic_write_json` — aman GUI-vs-CLI konkuren (ada test).
  - `metadata render` single/batch + `preview` kini memakai `EditorSession` (hapus ~120 baris duplikasi).
- **P2 watch & schema**:
  - `metadata watch --project --images -o [--interval 1] [--once]` — poll incremental, render ulang hanya halaman dirty (hash `bubbles`), JSONL ke stderr.
  - Skema: `Bubble.mask_path?`, `PageEditData.order?` (+ `ordered_bubbles()`, `validate_page()` cek `unknown_order_id`), `metadata edit --order "1,ft1,2" --mask-path "ID=path"`.

---

## [0.1.1-dev.13] - 2026-09-10

Commit: `f2a4ca3` — `feat: freetext outside bubbles + rapid ort real (vision/ft, auto-download, Indo proof)`

Branch: `dev-kz-debug`

### Added
- **Freetext Outside Bubbles (kzkt parity)**:
  - `src/preparer.rs` `detect_free_text()` `WHITE` mask `scale 2048` `canvas.drawRect` bubble → `OcrEngine.recognize_regions` filter `w<12||h<8` `len==1 alnum` `center insideBubble` `IoU>=0.3` `mergeNearby gap20` `freetext_pad 6%` `bg median`.
  - `src/ocr.rs` `TextRegion{bbox,text}` `OcrScript Japanese/English/Korean/Chinese/Auto` `trait OcrEngine {recognize, recognize_regions}` `NoopOcr` `RapidOcr {det_path,rec_path,dict_path}` `MangaOcr` `TesseractOcr {jpn_vert+jpn psm6 tsv conf>=30}` `create_ocr_engine()` `cache_dir_for ~/.cache/kzktdk/models/<engine>/` `ensure_rapid_cached()` auto-download `curl -L` `ch_PP-OCRv3_det_infer.onnx 2.4M + japan_rec_crnn.onnx 3.5M + japan_dict.txt 17K` `SWHL/RapidOCR` `~/.cache/kzktdk/models/rapid 5.8M` `ort 2.0.0-rc.13` `Session::builder().commit_from_file` letterbox `640` threshold `0.3` BFS boxes real `ft1 [374,277,616,378]` bukan dummy `[20,20]`.
  - CLI `translate/detect --ocr none|rapid|manga|tesseract|vision --translate-free-text --ocr-script jp --mode vision|ocr|auto --ocr-model <path>` `TranslationContext` `combined_dets ft1..` `crops` padded `raw_map` `vision mosaic` vs `ocr JSON` vs `auto` fallback `norm_map` lowercase `ft` `inpaint_targets combined` `Typesetter render_bubble_text_with_style` `metadata PageEditData ocr_engine ft conf0.90`.
  - `src/translation/mod.rs` prompt `RED ID numeric atau ft1/ft2 include ALL IDs even if SKIP` agar LLM tidak skip freetext.

### Fixed
- **Rapid Dummy → Real**: `RapidOcr` sebelumnya dummy `[20,20] テスト` `SKIP` sekarang `ort det` real `Sonico page01 ft1 [374,277,616,378] → Band beranggotakan tiga wanita...` `91%` pixel beda `ft1_render_proof` Indo proof `sonico01_indo.jpg`.
- **FT Uppercase**: `norm_map` lowercase `FT1→ft1` sebelum inpaint/typeset agar `all_translations` tidak miss.

### Changed
- `translate_page` `detect` single+batch `make_page_with_ft` `draw_rect green` `batch json` `project.kedit.json`.

---

## [0.1.1-dev.12] - 2026-09-10

Commit: `f9af774` — `feat: ocr prep — raw_text, bold/italic, batch style, backup`

Branch: `dev-kz-debug`

### Added
- **OCR Prep — Raw Text & Stub Engine**:
  - Menambah `Bubble.raw_text` dan `PageEditData/Project.ocr_engine` (`Option<String>`) dengan `serde default` agar JSON lama tetap load.
  - Modul `src/ocr.rs` `trait OcrEngine` + `NoopOcr` stub — pipeline tetap Vision-only, `raw_text` tetap `None` sampai engine dipilih.
  - Flag `translate --ocr <none|vision|local>` (dummy, log fallback none) dan `metadata edit --raw-text "ID=text"` + `preview --show-raw` untuk menampilkan `raw_text` tanpa save.
  - `metadata show` kolom `RAW` dan `ocr` di header, handling unicode `chars()` aman untuk Jepang.

- **Bold / Italic & Batch Style**:
  - `BubbleStyle { is_bold, is_italic }` + flag `metadata edit --bold/--italic "ID=true"` dan `--apply-style "align=center,bold=true,font_size=20"` ke semua bubble.
  - `--replace "old=new"` find & replace ke seluruh `translated` dengan `edited=true`.

- **Backup Lightweight Undo**:
  - `atomic_write_json` kini simpan `.kedit.json.bak.<ts>` sebelum overwrite, keep 3 terbaru — ganti `undo` GUI `kzkt` tanpa history in-memory.

### Changed
- `metadata export` single page tulis object (bukan array) agar `load_page_metadata` kompatibel; multi-page tetap array.

### Fixed
- `metadata show` panic pada `raw_text` unicode (slice byte `..10` → `chars().take(10)`).

---

## [0.1.1-dev.11] - 2026-09-10

Commit: `29823a9` — `feat: CLI editor final (pack/show/preview override/edit bg)`

Branch: `dev-kz-debug`

### Added
- **Metadata Show & Project List**:
  - `metadata show <json> [--id ID]` auto-detect `PageEditData` vs `project.kedit.json` — Page menampilkan tabel `ID | BBOX | CONF | EDIT | STYLE | TRANSLATED`, Project menampilkan daftar `pages` dengan bubble count.
  - Mendukung filter `--id` untuk fokus satu bubble dan validasi `width/height` bounds di output.

- **Pack to CBZ**:
  - `metadata pack <folder> -o chapter.cbz` mengemas folder hasil `render` menjadi CBZ dengan `natural sort` (1,2,10) dan penamaan `page_0001.jpg`.
  - Menangani folder kosong dengan pesan `No images found` yang jelas.

- **Batch Render from Project**:
  - `metadata render --project project.kedit.json --images <folder> -o rendered/ --jobs auto` merender seluruh halaman dari `project.kedit.json` tanpa LLM, dengan opsi `images` untuk override lokasi gambar asli dan `--jobs` untuk paralelisme.
  - Mode single `metadata render <image> --metadata page.kedit.json -o out.jpg` tetap kompatibel.

- **Preview Style Overrides**:
  - `metadata preview --id 1 --text "Halo" --font-size 28 --text-color 255,0,0 --align center` memungkinkan uji gaya tanpa menyimpan JSON, cocok untuk live editor.
  - Mendukung override `font-family`, `font-size`, `text-color`, `stroke-color`, `align`.

- **Edit Enhancements**:
  - Flag baru `--bg-color "ID=R,G,B"` dan `--conf "ID=0.99"` untuk koreksi warna background dan confidence detector.
  - `--clear-style "ID"` menghapus seluruh style per-bubble dalam satu perintah.
  - Validasi `bbox` kini memeriksa `x2<=width && y2<=height` serta `x1<x2`, mencegah koordinat di luar halaman.

- **CLI Discoverability**:
  - `kzktdk -h` kini menampilkan contoh `metadata` dan `font` serta daftar `Commands: translate, metadata, font`.

### Changed
- Signature `metadata render` dan `metadata preview` diperluas dengan flag opsional namun tetap backward compatible — panggilan lama tanpa flag baru tetap berjalan.
- `metadata validate` kini auto-detect `Project` vs `PageEditData` dan memberikan peringatan untuk `duplicate id` atau `bbox out of bounds` dengan `target_lang` dan `prompt_sig` yang lebih informatif.

### Fixed
- Help template sebelumnya hanya menampilkan `translate`; kini menampilkan alur editor lengkap (`export → edit → render/preview → show/pack`) sehingga pengguna menemukan fitur tanpa `grep` manual.

---

## [0.1.1-dev.10] - 2026-09-10

Commit: `853236e` — `fix: translate_page use with_style for consistency`

Branch: `dev-kz-debug`

### Fixed
- **Translate Pipeline Style Consistency**:
  - `translate_page` kini menggunakan `render_bubble_text_with_style` agar konsisten dengan `metadata render`, mempersiapkan dukungan style per-bubble di pipeline translate.

---

## [0.1.1-dev.9] - 2026-09-10

Commit: `c10a510` — `chore: update .gitignore and remove debug bins`

Branch: `dev-kz-debug`

### Changed
- **Gitignore & Debug Cleanup**:
  - Menambahkan `src/bin/test_*.rs` ke `.gitignore` dan menghapus binary debug sementara `test_deser.rs`/`test_deser2.rs`.

---

## [0.1.1-dev.8] - 2026-09-10

Commit: `33b9ec6` — `docs: update changelog hash for eb68297`

Branch: `dev-kz-debug`

### Fixed
- **Changelog Hash Correction**:
  - Memperbaiki hash `a9536cd → eb68297` di entri `0.1.1-dev.7` setelah amend.

---

## [0.1.1-dev.7] - 2026-09-10

Commit: `eb68297` — `fix: metadata render style override (font_size/color/align)`

Branch: `dev-kz-debug`

### Fixed
- **Metadata Render Style Overrides**:
  - `metadata render` sebelumnya hanya menghormati `font_family` sehingga `font_size`, `text_color`, `stroke_color`, dan `align` diabaikan — kini menerapkan `BubbleStyle` penuh per bubble dengan Typesetter kustom.
  - Menghapus log debug sementara setelah verifikasi.

---

## [0.1.1-dev.6] - 2026-09-10

Commit: `ff6225f` — `docs: changelog Tahap 1+2 font & style`

Branch: `dev-kz-debug`

### Changed
- **Changelog Per-Commit Tracking**:
  - Menambahkan entri `0.1.1-dev.4` (Tahap 1 font registry) dan `0.1.1-dev.5` (Tahap 2 style editing) untuk pelacakan per commit.

---

## [0.1.1-dev.5] - 2026-09-10

Commit: `093bc28` — `feat: bubble style editing + preview (Tahap 2)`

Branch: `dev-kz-debug`

### Added
- **Bubble Style Editing & Preview**:
  - Menambahkan `BubbleStyle` (`font_size`, `text_color`, `stroke_color`, `align`) dan flag `edited` untuk tracking manual.
  - `metadata edit --font-size/--text-color/--stroke-color/--align/--edited` untuk mengedit gaya per bubble.
  - `metadata preview <image> --metadata --id --text -o preview.jpg` merender satu bubble dalam <100ms tanpa menyimpan JSON — siap untuk live editor.
  - `Typesetter` kini mendukung `font_size` override (clamp min/max), warna teks/stroke, dan perataan `left/center/right`.

---

## [0.1.1-dev.4] - 2026-09-10

Commit: `a3671f2` — `feat: font registry global+per-bubble (Tahap 1)`

Branch: `dev-kz-debug`

### Added
- **Font Registry Global & Per-Bubble**:
  - `FontRegistry` (`import/list/remove/resolve`) mengelola pool `~/.local/share/kzktdk/fonts` dan `AppConfig` (`~/.config/kzktdk/config.toml`) untuk default Latin/CJK.
  - `font` subcommand: `import`, `list`, `remove`, `set-default`, `get-default` dengan validasi `FontArc`.
  - `translate --font <name>` kini menerima nama dari registry selain path file.
  - `metadata edit --font-family` dan `metadata render` per-bubble font override via Typesetter kustom.

---

## [0.1.1-dev.3] - 2026-09-10

Commit: `a7f7c4e` — `feat: metadata add/delete box`

Branch: `dev-kz-debug`

### Added
- **Bubble Add/Delete**:
  - `metadata edit --add "ID=x1,y1,x2,y2[=text]"` menambah bubble baru dengan validasi duplikat dan `x1<x2 && y1<y2`.
  - `metadata edit --delete "ID"` menghapus bubble berdasarkan ID.
  - Mendukung teks mengandung `=` dan memberikan log `Added/Deleted/already exists/not found`.

---

## [0.1.1-dev.2] - 2026-09-10

Commit: `7c850b2` — `feat: CLI editor-ready metadata separation`

Branch: `dev-kz-debug`

### Added
- **Editor-Ready Metadata Separation**:
  - `PageEditData`/`Project` (`version`, `page`, `width/height`, `target_lang`, `prompt_sig`, `bubbles`) dengan `atomic_write_json` dan `save/load_page_metadata`/`save_project`.
  - `translate --save-metadata` menyimpan sidecar `.kedit.json` per halaman + `project.kedit.json` (single & batch `JoinSet` parallel).
  - `metadata` subcommand: `export` (detect tanpa LLM, murah), `render` (inpaint+typeset tanpa LLM), `edit --set/--bbox`, `validate`.
  - `detect` batch `folder/cbz/epub` via `prepare_input`, `--json`, `--jobs auto` dengan `natural sort`.

---

## [0.1.1-dev.1] - 2026-09-09

Commit: `c35ad0d` — `feat: port cache, inpaint parallel, batch parallel, fallback, rate-limit, repair, epub`

Branch: `dev-kz-debug` (dari `master ceda763`)

### Added
- **Translation Cache & Performance**:
  - `TranslationCache` (`rusqlite` + `blake3`, `prompt_signature` custom/classic) di `~/.cache/kzktdk/cache.db` dengan `--no-cache`/`--clear-cache`.
  - `ProviderChain` fallback (`--fallback-provider`) dan `RateLimiter` (`--rate-limit 3`, backoff 1s/2s/4s) dengan `repair_json_output`.
  - `inpaint_image` paralel via `rayon` dan `translate` batch paralel `JoinSet + Semaphore(jobs=auto)`.
  - Dukungan `EPUB` sebagai ZIP dengan skip `META-INF` dan `natural sort`.

---

## [0.1.1] - 2026-09-10

Commit: `c762821` — `chore: update Cargo.lock`

Branch: `dev-kz-debug`

### Changed
- Memperbarui `Cargo.lock` untuk `serde_derive`.

---

## [0.1.0] - 2026-09-09

Commit: `ceda763` — `feat: initial release of kzktdk`

### Added
- Port Rust awal dari `kouzen-neo/kzkt` Android — `YoloModel` cascade 3-stage, `inpaint` Telea Rust, `MosaicBuilder`, `Typesetter` eliptik, `archive` natural sort.
- CLI: `translate`, `detect`/`inpaint`/`decrypt-model` (hidden) dengan `fonts/Komika Axis` & `KosugiMaru`.

### Docs
- `README.md`, `DOCUMENTATION.md`, `CONTRIBUTING.md`.
