# Prompt: OCR Prep — Struktur Sebelum Engine (dev-kz-debug 0.1.1-dev.12)

> JANGAN EKSEKUSI — hanya prompt. Eksekusi nanti via `PROMPT_OCR_PREP.md` di branch `dev-kz-debug`.

## Objective
Siapkan fondasi `raw_text` / `OcrEngine` agar nanti tinggal colok `tesseract` atau `Vision OCR` tanpa migrasi breaking. Perilaku `translate` sekarang (Vision-only) harus tetap identik — `raw_text` tetap `None`.

## Branch & Version
- Branch: `dev-kz-debug` (HEAD `6f9bca9`)
- Version bump: `0.1.1-dev.12` di `CHANGELOG.md`
- Commit: `docs+feat: ocr prep (model+stub+cli dummy+non-ocr P0)` — per-task commit kecil opsional

## Task 1 — Model Forward-Compatible `src/metadata.rs:7,21,34`
- Tambah field:
  ```rust
  Bubble { raw_text: Option<String> } // #[serde(default)]
  PageEditData { ocr_engine: Option<String> } // #[serde(default)] "none"
  Project { ocr_engine? } jika ada
  BubbleStyle tetap { font_family, font_size, text_color, stroke_color, align }
  ```
- `load_page_metadata` / `load_project` harus `#[serde(default)]` + `deny_unknown_fields=false` implisit — JSON lama tanpa field tetap load.
- `save_page_metadata` `atomic_write_json:72` tulis `raw_text` hanya jika `Some` (`skip_serializing_if Option::is_none`) agar file lama tidak membengkak.
- Tambah unit test `cargo test` case: load `dev.11` json tanpa `raw_text` → `raw_text==None`.

## Task 2 — Abstraksi Stub `src/ocr/mod.rs` (baru) + wiring `src/main.rs:1606`
- Buat modul:
  ```rust
  // src/ocr/mod.rs
  pub trait OcrEngine { fn recognize(&self, image: &image::RgbImage, bbox:[u32;4]) -> Option<String>; }
  pub struct NoopOcr;
  impl OcrEngine for NoopOcr { fn recognize(&self, _: &RgbImage, _: [u32;4]) -> Option<String> { None } }
  ```
- `src/lib.rs` export `pub mod ocr;`
- Wiring `translate_page` `src/main.rs:1506,1606` — inject `ocr: &dyn OcrEngine` (sekarang `&NoopOcr`), log `if let Some(raw)=ocr.recognize(...) { bubble.raw_text=Some(raw) }` else Vision path existing — jangan ubah `ProviderChain` `src/translation/mod.rs:408`.
- Verifikasi: `kzktdk translate <image> --save-metadata` → `.kedit.json` `raw_text: null` atau `absent`, terjemahan tetap sama md5.

## Task 3 — CLI Flag Dummy `src/main.rs:126,251,289`
- `translate --ocr <none|vision|local>` default `none` `value_parser ["none","vision","local"]` — jika `local|vision` → `eprintln!("[OCR] engine '{}' belum diimplementasikan, fallback none", ocr)` dan pakai `NoopOcr`.
- `metadata edit --raw-text "ID=text"` `empty=clear` mirip ` --font-family` `src/main.rs:306` — set `b.raw_text`.
- `metadata preview --show-raw` bool — jika `raw_text Some` tampilkan `raw_text` bukan `translated` untuk debug (tanpa save).
- `metadata show` tabel tambah kolom `RAW` jika ada.
- Jangan install `libtesseract` — flag hanya plumbing.

## Task 4 — P0 Non-OCR (bisa paralel, tidak butuh OCR)
### 4a Auto BG
- Implement `detect_bubble_bg(image,bbox) -> [u8;3]` sampling median luminance `0.299R+0.587G+0.114B` seperti `DOCUMENTATION.md:3.1` — di `src/inpaint/mod.rs` atau `src/typesetting/mod.rs:399` fallback: `bg = bubble.bg_color.unwrap_or_else(|| detect(image,bbox))` untuk `render`/`preview` kontras `is_dark_bg`.

### 4b Bold/Italic
- `BubbleStyle { is_bold: Option<bool>, is_italic: Option<bool> }` `src/metadata.rs:7` + flag `metadata edit --bold/--italic "ID=true/false"` `empty=clear` — `Typesetter::render_bubble_text_with_style` `src/typesetting/mod.rs:325` handle `is_bold` via `stroke 0.3px` atau font weight, `is_italic` via `skew`.

### 4c Batch Style
- `metadata edit --replace "old=new"` + ` --apply-style "align=center,bold=true,scale=1.2"` iterasi semua `bubbles` `src/main.rs:1267` — mirip `BatchEditDialog.kt:139`.

### 4d Backup (ganti undo)
- `atomic_write_json` `src/metadata.rs:78` sebelum `rename` copy `path -> path.bak.<timestamp>` (max 3) agar `undo` manual tanpa GUI history `InteractiveEditorDialog.kt:97`.

## Verifikasi (wajib sebelum commit)
```bash
cargo build
cargo test # raw_text missing field test
./target/debug/kzktdk translate ~/Downloads/Inconvenient.../01_original_page01.jpg --save-metadata -o /tmp/ocr_prep --ocr none && cat /tmp/ocr_prep/*.kedit.json | grep raw_text
./target/debug/kzktdk metadata edit /tmp/ocr_prep/*.kedit.json --raw-text "1=テスト" && ./target/debug/kzktdk metadata show /tmp/ocr_prep/*.kedit.json | grep RAW
./target/debug/kzktdk metadata preview <img> --metadata <json> --id 1 --show-raw -o /tmp/preview_raw.jpg && md5sum /tmp/preview_raw.jpg
```
- md5 translate sebelum vs sesudah patch — harus identik (Noop).
- `CHANGELOG.md` tambah `0.1.1-dev.12` entry kzkt-style bold titles, `Branch: dev-kz-debug`.

## Non-Goals
- Jangan install `tesseract`/` Tessdata`, jangan panggil API OCR kedua, jangan ubah `Cargo.toml` deps berat — hanya plumbing.
- Jangan ubah pipeline Vision mosaic `src/translation/mod.rs` — Vision tetap satu-satunya sumber terjemahan sampai engine dipilih.

## Output
- Satu commit (atau 4 commit kecil) di `dev-kz-debug` dengan `CHANGELOG.md` `0.1.1-dev.12` + file `src/ocr/mod.rs` + patch `src/metadata.rs` `src/main.rs` `src/lib.rs`.
