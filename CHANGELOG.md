# Changelog

Semua perubahan penting pada proyek ini didokumentasikan di file ini.

Format berdasarkan [Keep a Changelog](https://keepachangelog.com/id/1.0.0/),
dan proyek ini mengikuti [Semantic Versioning](https://semver.org/lang/id/).

## [Unreleased]

### Rencana
- PDF support (`pdfium-render`) — `PDF in → PDF out` (ditunda)
- Tauri v2 GUI skeleton (Svelte + Konva.js)
- Hardware acceleration (DirectML/CoreML/CUDA)

---

## [0.1.1] - 2026-09-10

Branch: `dev-kz-debug` — `c762821`

### Changed
- `Cargo.lock` update untuk `serde_derive` (fix `serde` derive macro)

---

## [0.1.1-dev.5] - 2026-09-10

Commit: `093bc28` — `feat: bubble style editing + preview (Tahap 2)`

Branch: `dev-kz-debug`

### Added
- `src/metadata.rs`: `BubbleStyle {font_family,font_size,text_color,stroke_color,align}`, `Bubble {style,edited}` dengan `#[serde(default)]`
- `src/typesetting/mod.rs`: `render_bubble_text_with_style` handle `font_size` override (clamp min/max, wrap), `text_color`/`stroke_color` override, `align` left/center/right (`line_x` logic), `font_family` via per-bubble Typesetter di `metadata render`/`preview`
- `src/main.rs`: `metadata edit` flag `--font-size "ID=14.5"`, `--text-color "ID=R,G,B"`, `--stroke-color`, `--align`, `--edited "ID=true/false"`, subcommand `metadata preview <image> --metadata --id --text -o preview.jpg` (single bubble inpaint+render <100ms, live editor)

### Tested
- `font_size 20` + `text_color 255,0,0` + `stroke 0,0,255` + `align center` + `edited true` → validate & render 178K
- `preview --id 1 --text "Preview Live"` → 180K single bubble, font_size override info
- `translate --font TestKomika` global via registry → 178K
- `font` + `metadata edit --font-family` tetap works

---

## [0.1.1-dev.4] - 2026-09-10

Commit: `a3671f2` — `feat: font registry global+per-bubble (Tahap 1)`

Branch: `dev-kz-debug`

### Added
- `src/font.rs` baru: `FontRegistry import/list/remove/resolve`, `AppConfig set/get-default` (`~/.local/share/kzktdk/fonts` + `fonts.json` + `~/.config/kzktdk/config.toml`), `FontArc` validasi, `toml 0.8`
- `src/metadata.rs`: `BubbleStyle {font_family}`, `Bubble {style,edited}`
- `src/typesetting/mod.rs`: per-bubble font via `FontRegistry::resolve` (custom Typesetter per bubble di `metadata render`)
- `src/lib.rs`: `pub mod font`
- `src/main.rs`: `font` subcommand `import/list/remove/set-default/get-default`, `translate --font` bisa `name` dari pool (bukan cuma path), `metadata edit --font-family "ID=FontName"`, `metadata render` per-bubble font override

### Tested
- `font import Komika` → `font list` → `font set-default`/`get-default`
- `metadata edit --font-family "1=WildWords"` → `render` per-bubble
- `translate --font TestKomika` global via registry

---

## [0.1.1-dev.3] - 2026-09-10

Commit: `a7f7c4e` — `feat: metadata add/delete box`

Branch: `dev-kz-debug`

### Added
- `metadata edit` flag `--add "ID=x1,y1,x2,y2[=text]"` — tambah bubble baru (validasi duplikat, bbox `x1<x2 && y1<y2`, 4 `u32`)
- `metadata edit` flag `--delete "ID"` — hapus bubble by ID (`retain`)
- Support teks mengandung `=` (split di `=` pertama setelah bbox dengan 3 koma)
- Log: `Added bubble 10 bbox [300,100,400,200] text "..."`, `Deleted bubble 10`, `already exists` / `not found`

### Tested
- Add `10=300,100,400,200=Halo Box Baru` → validate 10 bubbles → render 178K
- Add tanpa teks `10=50,50,150,150` → translated `""`
- Delete, duplicate, missing ID handling
- Hasil: `render_with_new_box.jpg` di `kzktdk_test_results/`

---

## [0.1.1-dev.2] - 2026-09-10

Commit: `7c850b2` — `feat: CLI editor-ready metadata separation`

Branch: `dev-kz-debug`

### Added
- `src/metadata.rs` baru: `Bubble {id,bbox,conf,translated,bg_color}`, `PageEditData {version,page,width,height,target_lang,prompt_sig,bubbles}`, `Project {version,pages,target_lang}`, `atomic_write_json` (`.tmp` + `rename`), `save/load_page_metadata`, `save/load_project`, `build_page_data`
- `src/lib.rs`: `pub mod metadata`
- `Cargo.toml`: `serde` feature `derive`
- `Commands::Detect` batch: `input` folder/cbz/epub via `archive::prepare_input`, `output` file/folder, `--json`, `--jobs auto` (natural sort, per-page `draw_rect`, batch JSON array)
- `Commands::Translate` flags: `--save-metadata`, `--metadata-dir` → sidecar `.kedit.json` per halaman + `project.kedit.json`, integrasi di `translate_page` (single & batch sequential+parallel `JoinSet`)
- `Commands::Metadata` subcommand:
  - `metadata export <input> --json out.json` — detect tanpa LLM (cheap)
  - `metadata render <image> --metadata page.kedit.json -o out.jpg` — inpaint+typeset dari JSON tanpa LLM
  - `metadata edit <json> --set "ID=text" --bbox "ID=x1,y1,x2,y2"` — edit JSON
  - `metadata validate <json>` — cek `PageEditData v1: N bubbles`

### Tested
- `detect /tmp/sonico_sample -o /tmp/detect_batch_test --json /tmp/detect_batch.json --jobs 2` → 2 preview + JSON 3.4K
- `translate --save-metadata` single: `/tmp/with_meta.kedit.json` 2.1K + `/tmp/project.kedit.json`
- `translate --save-metadata` batch 2 halaman: `batch_meta_test/*.kedit.json` + `project.kedit.json` 276B
- `metadata render` tanpa LLM → 178K, `edit` + `validate` sukses

---

## [0.1.1-dev.1] - 2026-09-09

Commit: `c35ad0d` — `feat: port cache, inpaint parallel, batch parallel, fallback, rate-limit, repair, epub`

Branch: `dev-kz-debug` (dari `master ceda763`)

### Added
- `Cargo.toml`: `blake3 =1.8.2`, `rusqlite =0.32.1 (bundled)`
- `src/cache.rs` baru: `TranslationCache` dengan `Mutex<Connection>` + `Send/Sync`, `blake3` hash `image_hash`, `prompt_signature` (`custom_` + 8 hex atau `classic`), `open_at`/`open_in_memory`, `get`/`put`/`clear`, `filter_cached`/`save_batch`, DB di `~/.cache/kzktdk/cache.db`
- `src/translation/mod.rs`: `Provider::name`/`model_name`, `translate_mosaic_raw`, `translate_text` (repair), `ProviderChain`, `RateLimiter {max_rps, retry_max, execute_with_retry (backoff 1s/2s/4s)}`, `translate_with_chain` + `repair_json_output` (prompt `Fix ONLY JSON... KEEP {lang}`)
- `src/inpaint/mod.rs`: `inpaint_image` parallel via `rayon::par_iter_mut` (extract crops → parallel `inpaint_crop` → copy back sequential)
- `src/archive.rs`: `is_epub_path`, `is_archive_path` tambah `epub`, `extract_archive` skip `META-INF/` & `mimetype`
- `src/main.rs`: `ProviderChain` + `RateLimiter` di `Translate`, CLI `--fallback-provider`, `--rate-limit 3`, `--jobs auto` (`available_parallelism`), `--no-cache`/`--clear-cache`, `TranslationContext` tambah `save_metadata`/`metadata_dir`, batch parallel `JoinSet + Semaphore(jobs)` dengan per-task `YoloModel` + per-task `TranslationCache::open()`, cache `Mutex`, input `Option<PathBuf>` untuk `--clear-cache`, `parse_jobs`
- `src/lib.rs`: `pub mod cache`

### Fixed
- `cargo check` `str_as_str` unstable (`as_slice` pada `&str`), `output` move borrow

### Tested
- `detect` 9 bubbles page 01, `inpaint` parallel 1.1M, `decrypt-model` 98MB
- `translate` single 9 bubbles → 178K (Gemini 3.1 Flash Lite)
- `translate` batch 2 pages `--jobs 2` cache hit 9/9, batch 20 pages `--jobs auto(8)` + `rate-limit` + `fallback` → CBZ 2.8M 20 files

---

## [0.1.0] - 2026-09-09

Commit: `ceda763` — `feat: initial release of kzktdk`

### Added
- Initial Rust port dari `kouzen-neo/kzkt` Android
- `YoloModel` 3-stage cascade `[(0.28,0.45),(0.18,0.55),(0.10,0.65)]`, `letterbox 640x640`, `NMS 0.45`, `remove_false_giants` (0.80 coverage, 6x area, 80% intersect), `remove_nonsense`, `merge_overlapping` (IoU 0.50 / coverSmall 0.85)
- `inpaint` Telea pure Rust (`inpaint` crate), `morph_close` r=15..45, interior scoring `area*(1-dist/max*0.5)`
- `translation` `MosaicBuilder` vertical `margin 70`, `Typesetter` elliptical `max(0.82, sqrt(max(0.20,1-0.40*y^2)))`, hyphenator EYD
- `archive` natural sort `natord`, `CBZ` pack `page_0001`
- CLI: `translate`, `detect` (hidden), `inpaint` (hidden), `decrypt-model` (hidden)
- Docs: `README.md`, `DOCUMENTATION.md`, `CONTRIBUTING.md`
