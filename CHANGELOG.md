# Changelog

Semua perubahan penting pada proyek ini didokumentasikan di file ini.

Format berdasarkan [Keep a Changelog](https://keepachangelog.com/id/1.0.0/),
dan proyek ini mengikuti [Semantic Versioning](https://semver.org/lang/id/).

## [Unreleased]

### Rencana
- Tauri v2 GUI skeleton (Svelte + Konva.js)
- Hardware acceleration (DirectML/CoreML/CUDA)

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
