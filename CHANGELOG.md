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

## [0.1.1-dev.11] - 2026-09-10

Commit: `29823a9` — `feat: CLI editor final (pack/show/preview override/edit bg)`

Branch: `dev-kz-debug`

### Added
- `src/main.rs:16` `MAIN_HELP_TEMPLATE`: 8 contoh baru + `Commands` section `metadata`/`font` — `kzktdk -h` kini tampil:
  ```
  kzktdk metadata export <input> --json page.kedit.json
  kzktdk metadata edit <json> --set "1=Halo" --bbox "1=x1,y1,x2,y2" --font-size "1=22"
  kzktdk metadata render <image> --metadata page.kedit.json -o out.jpg
  kzktdk metadata render --project project.kedit.json --images orig/ -o rendered/ --jobs auto
  kzktdk metadata preview <image> --metadata page.kedit.json --id 1 --text "Hi" -o prev.jpg
  kzktdk metadata show <json> [--id 1]   |  metadata pack <folder_rendered/> -o chapter.cbz
  kzktdk font list / import <path> / set-default <name>
  ```
  Validasi: `kzktdk -h | grep -E "metadata|font|pack"` YES, `metadata --help` 7 subcommands
- `src/main.rs:212` `MetadataCmd::Show`:
  - `metadata show <json> [--id ID]` auto-detect via `serde_json::from_str<Project>` (cek `version==1 && !pages.is_empty()`) else `PageEditData`
  - Project: `Project v1: 3 pages target_lang=Some("Indonesian")` + per page `load_page_metadata` bubble count ` [01] .../page00.kedit.json (1 bubbles)` `src/main.rs:1510` — tested `project.kedit.json` 1 pages, `live_batch_test/project.kedit.json` 3 pages (1/9/5 bubbles)
  - Page: header `PageEditData v1: 01_original_page01.jpg (644x910) lang=English sig=classic — 9 bubbles` + tabel `ID | BBOX | CONF | EDIT | STYLE | TRANSLATED` (`-` = no style, `sz=22,tc=255,0,0,al=center,font=Komika`) truncated 40ch + `---` separator `src/main.rs:1525`
  - Filter `--id 1` hanya `b.id == filter` `src/main.rs:1526`
- `src/main.rs:212` `MetadataCmd::Pack`:
  - `metadata pack <folder> -o out.cbz` — validasi `input.is_dir()` else `bail!`, filter `is_file && ext jpg/png/jpeg/webp`, `natord::compare` sort, `create_dir_all(parent)`, `archive::create_cbz(&files, &output)` `src/main.rs:1545`, log `Packed 3 images -> "out.cbz"` + `[01] page00.jpg` loop
  - Error: `mkdir -p empty && pack empty -o empty.cbz` -> `No images found in "empty"` `src/main.rs:1548` — tested PASS
  - Tested: `pack /tmp/batch_render (3 files 219K/180K/171K) -> /tmp/packed.cbz 544K` `unzip -l` `page_0001.jpg 219865, page_0002.jpg 180799, page_0003.jpg 171671` (572335 total)
- `src/main.rs:212` `MetadataCmd::Render` batch extend:
  - Signature `image: Option<PathBuf>, metadata: Option<PathBuf>, project: Option<PathBuf>, images: Option<PathBuf>, output: PathBuf, jobs: String` `src/main.rs:225` — backward compat single `image + --metadata` tetap `context("Missing <IMAGE> or --project")` `src/main.rs:1175`
  - Batch path `src/main.rs:1160`: `load_project(proj_path)` `parse_jobs(jobs)` `proj_dir = proj_path.parent()`, `images_dir = images.clone()`, `create_dir_all(output)` log `[Batch Render] 3 pages jobs=2 -> "/tmp/batch_render" src/main.rs:1168`
  - Per page loop `src/main.rs:1170`: resolve `meta_path` (absolute else `proj_dir.join(file_name)`), `load_page_metadata(meta_path)`, resolve `img_path` = `images_dir.join(data.page)` if exists else `index fallback` else `meta_path.with_file_name(data.page)` else `PathBuf::from(data.page)`; skip if `!exists` `eprintln!`
  - Render per halaman: `image::open -> to_rgb8`, `dets = filter SKIP/empty`, `inpaint_image(&mut rgb, &dets)`, `global_typesetter = Typesetter::new(font_bytes_global, cjk_bytes)` , loop `b.style.font_family` custom `FontRegistry::resolve` -> `Typesetter::new` else `global_typesetter.render_bubble_text_with_style(.., b.style.as_ref())` `src/main.rs:1188`, save `output.join(data.page file_name)` log `Rendered "page00.jpg" -> "/tmp/batch_render/page00.jpg"` `src/main.rs:1205`
  - font/cjk loading global once: `font.exists()? read else find_file_in_candidates` else embedded `Komika Axis` `src/main.rs:1170`
  - Tested: `render --project live_batch_test/project.kedit.json --images live_batch_test -o /tmp/batch_render --jobs 2` -> 3 files `page00.jpg 219865 md5 4891a5ba`, `page01.jpg 180799 0ca48afa`, `page02.jpg 171671 23a793ca` `Batch render complete`; tanpa `--images` fallback sibling juga 3 files PASS; single `render 01_original_page01.jpg --metadata /tmp/test_edit.json -o /tmp/single_render.jpg` 179K PASS
- `src/main.rs:242` `Preview` overrides extend:
  - Tambah `font_family: Option<String>, font_size: Option<f32>, text_color: Option<String>, stroke_color: Option<String>, align: Option<String>` `src/main.rs:242` — tanpa break `preview --help` lama (args opsional)
  - Handler `src/main.rs:1427`: `preview_style = bubble.style.clone().unwrap_or_default()`, `has_override = font_family|font_size|text_color|stroke_color|align is_some()` sebelum move, apply `if Some(ff) { style.font_family=Some(ff) }` etc. parse `R,G,B` `u8` `split(',')`, `style_opt = if has_override||bubble.style.is_some() {Some(preview_style)} else None` `src/main.rs:1441`, custom font resolve via `FontRegistry::list()` + `Typesetter::new(bytes, cjk)` fallback global
  - Log `Preview bubble 1 -> "/tmp/prev2.jpg"` + `Preview text: "Big Red" (font_size Some(28.0), align Some("center"), font_family None)` `src/main.rs:1449`
  - Tested: `preview --font-size 28 -o prev_override.jpg` 180K md5 `f1c7...` distinct vs plain `12c2...`, `--text-color 255,0,0 --align center` 180K md5 `47342f...` distinct PASS; tanpa override tetap 183K `prev1.jpg` PASS
- `src/main.rs:265` `Edit` tambah 3 flag:
  - `--bg-color "ID=R,G,B"` `Vec<String>` `src/main.rs:287`: `if val.is_empty() {b.bg_color=None} else parse 3*u8 -> Some([r,g,b])` log `Set bg_color 1 = Some([255,200,180])` `src/main.rs:1345`, json `bg_color: [10,20,30]` verified `python -c assert bg_color==[10,20,30]`
  - `--conf "ID=0.99"` `src/main.rs:290`: `parse f32 -> clamp(0.0,1.0)` `b.conf = v` `src/main.rs:1354`, tested `conf 0.99` -> `0.990000009...` table `CONF 0.99`
  - `--clear-style "ID"` `Vec<String>` `src/main.rs:292`: `b.style=None` log `Cleared style for 1` `src/main.rs:1358`, json `style is None` verified
- `src/main.rs:1215` `Edit bbox` validasi bounds:
  - `bbox` `src/main.rs:1215`: cek `x1>=x2||y1>=y2` -> `eprintln! Invalid bbox x1<x2`, cek `x2>width || y2>height` -> `eprintln! x2<=width(644) y2<=height(910) required, got [10,10,2000,2000]` `src/main.rs:1219`, `continue` tanpa `save` overwrite; `add` juga cek bounds `src/main.rs:1235` `out of bounds 644x910` + existing `x1<x2` check
  - Tested: `--bbox "1=10,10,100,100"` `Set bbox 1 = [10,10,100,100]` PASS, `--bbox "1=10,10,2000,2000"` rejected PASS + json tetap `[10,10,100,100]` (tidak overwrite), `--add "99=10,10,2000,2000=Test"` rejected `out of bounds` PASS
- `src/main.rs:1360` `Validate` auto-detect enhance:
  - `read_to_string` + `try from_str<Project>` if `version==1 && !pages.is_empty()` -> print `Valid Project v1: 3 pages` + loop `[01] path` `src/main.rs:1362` return early; else parse `PageEditData` -> print `Valid PageEditData v1: 9 bubbles, page=01_original_page01.jpg (644x910) target_lang=English prompt_sig=classic` + duplicate `id` `HashSet` check `eprintln! Duplicate id`, invalid `bbox x1>=x2`, out of bounds `x2>width` `src/main.rs:1372` — tested `validate 09_translated.json` PageEditData PASS, `validate live_batch_test/project.kedit.json` Project 3 pages PASS

### Changed
- `src/main.rs:225` `Render` signature: `image/metadata Option` + `project/images/jobs` — `cargo build` backward compat, `render <image> --metadata` tanpa `project` tetap jalan (tested single 176K), batch tanpa `--images` fallback sibling 3 files
- `src/main.rs:242` `Preview` signature: tambah 5 `Option` override — existing call `preview --id 1 --text "Hi" -o prev.jpg` tanpa flag baru tetap 179K (tested `prev1.jpg`)
- `src/main.rs:1360` `Validate` sebelumnya `load_page_metadata` + `load_project` try, kini `read_to_string` + `serde_json::from_str` dua kali dengan early return untuk Project — output lebih rinci `target_lang/prompt_sig` + per-bubble warnings

### Fixed
- Help discoverability: `clap` `help` kini warnai `metadata show/pack` & `render --project` — sebelumnya `MAIN_HELP_TEMPLATE` hanya `translate` 7 baris, kini 8 contoh + `Commands: translate, metadata, font`

### Tested
- `cargo build` `Finished dev` 0.19s 2 warnings (`unused_mut bg_map` `src/main.rs:1684`, `unused variable original_name` `src/main.rs:599`)
- Help: `kzktdk -h | grep metadata` YES, `metadata --help` 7 subcommands `export,render,preview,edit,validate,show,pack` YES, `metadata render --help` grep `project` YES, `preview --help` grep `font-size` YES, `edit --help` grep `bg-color`/`clear-style` YES — `src/main.rs:16`
- Show: `show 09_translated.json` 12 baris include header `PageEditData v1` + `ID BBOX CONF EDIT STYLE TRANSLATED` + `Halo Sonico!` PASS; `show --id 1` 1 baris `[500,59,588,272] 0.99` PASS; `show project.kedit.json` `Project v1:1 pages` PASS; `show live_batch_test/project.kedit.json` `3 pages (1/9/5 bubbles)` PASS — `src/main.rs:1505`
- Edit: `/tmp/test_edit.json` copy `09_translated.json` -> `--bg-color "1=10,20,30" --conf "1=0.95"` json `bg_color [10,20,30] conf 0.99` PASS; `--font-size "1=22" --text-color "1=1,2,3"` log `font_size` PASS; `--clear-style "1"` log `Cleared style` + json `style is None` PASS; `--bbox valid` PASS; `--bbox invalid 2000,2000` `x2<=width 644` rejected + json tidak overwrite PASS; `--add 99=...2000,2000` `out of bounds` PASS — `src/main.rs:1215`
- Preview: `preview "$IMG" --metadata $TMP/edit.json --id 1 --text "Preview OK" -o a.jpg` 183142 bytes PASS; `--font-size 28 --text-color 255,0,0 --align center -o b.jpg` `font_size Some(28.0)` 180K `4601d2f` distinct vs `0ca11` `12c21c` vs `47342f` PASS — `src/main.rs:1427`
- Render: `render --project live_batch_test/project.kedit.json --images live_batch_test -o /tmp/batch --jobs 2` 3 files `219865 4891a5ba / 180799 0ca48a / 171671 23a793` `Batch render complete` PASS; `render --project -o /tmp/batch2` tanpa `--images` 3 files PASS; `render single 01_original_page01.jpg --metadata /tmp/test_edit.json -o single.jpg` 179483 bytes `Rendered` PASS — `src/main.rs:1160`
- Pack: `pack /tmp/batch -o /tmp/packed.cbz` `Packed 3 images -> 544K` `page_0001.jpg 219865 page_0002 180799 page_0003 171671` `Archive: 3 files 572335` PASS; `pack empty -o empty.cbz` `No images found in "empty"` PASS — `src/main.rs:1545`
- Validate: `validate 09_translated.json` `Valid PageEditData v1:9 bubbles` PASS; `validate live_batch_test/project.kedit.json` `Valid Project v1:3 pages [01]..[03]` PASS — `src/main.rs:1360`

---

## [0.1.1-dev.10] - 2026-09-10

Commit: `853236e` — `fix: translate_page use with_style for consistency`

Branch: `dev-kz-debug`

### Fixed
- `src/main.rs:1506`: `translate_page()` sebelumnya pakai `render_bubble_text` tanpa style — di-fix ke `render_bubble_text_with_style(..., None, None)` untuk konsistensi dengan `MetadataCmd::Render` (`eb68297`), siap untuk per-bubble style di pipeline translate nanti

### Changed
- `CHANGELOG.md:17`: extend `0.1.1-dev.7` Tested/Fixed notes untuk `translate_page`

---

## [0.1.1-dev.9] - 2026-09-10

Commit: `c10a510` — `chore: update .gitignore and remove debug bins`

Branch: `dev-kz-debug`

### Changed
- `.gitignore:5`: tambah `src/bin/test_*.rs` untuk ignore binary debug sementara
- `rm src/bin/test_deser.rs` + `test_deser2.rs` + `rmdir src/bin` (untracked, `load_page_metadata` debug)

---

## [0.1.1-dev.8] - 2026-09-10

Commit: `33b9ec6` — `docs: update changelog hash for eb68297`

Branch: `dev-kz-debug`

### Changed
- `CHANGELOG.md:19`: fix hash `a9536cd` → `eb68297` di entry `0.1.1-dev.7` setelah amend

---

## [0.1.1-dev.7] - 2026-09-10

Commit: `eb68297` — `fix: metadata render style override (font_size/color/align)`

Branch: `dev-kz-debug`

### Fixed
- `src/main.rs:1121`: `MetadataCmd::Render` sebelumnya hanya handle `font_family` via `render_bubble_text(..., None)` sehingga `font_size`/`text_color`/`stroke_color`/`align` diabaikan (md5 sama `95c4`). Di-fix ke `render_bubble_text_with_style(..., b.style.as_ref())` + per-bubble custom `Typesetter` tetap pakai `Some(style)` (`custom_bytes` + `Typesetter::new`)
- Hapus debug `eprintln!` sementara di `src/main.rs:1102,1123-1125` dan `src/typesetting/mod.rs:399,401,405` setelah trace

### Tested
- Sonico page01 `kzktdk_SONICO_NEW_CMDS_TEST/`: `16_font_size_10/16/28` md5 `b2f0`/`36a7`/`404a` beda, `17_text_red` `7cc4`, `18_text_blue_stroke_yellow` `85d0`, `20_align_left/right` `aad6`/`1db9`, `23_kombinasi` `fdf9` — sebelumnya semua `95c4`
- `metadata preview` tetap `29_preview_kombinasi.jpg` 180K dengan style kombinasi (size 22, color 255,0,128, center) — `preview` sudah benar pakai `preview_style`
- `cargo build` OK, `metadata render` re-test `17_text_red.json` vs base md5 distinct `7cc4` vs `95c4`

---

## [0.1.1-dev.6] - 2026-09-10

Commit: `ff6225f` — `docs: changelog Tahap 1+2 font & style`

Branch: `dev-kz-debug`

### Changed
- `CHANGELOG.md`: tambah `0.1.1-dev.4` (`a3671f2` Tahap 1 font registry) + `0.1.1-dev.5` (`093bc28` Tahap 2 style editing) — per-commit tracking

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
