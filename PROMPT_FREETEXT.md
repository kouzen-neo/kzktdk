# Prompt — Freetext Outside Bubbles (kzkt parity)

Tujuan: port `PagePreparer.detectFreeText` + `LocalOcrEngine.recognizeTextRegions` agar `--translate-free-text` ikut menerjemahkan narasi/caption/SFX di luar bubble YOLO, dengan ID `ft1, ft2` dan mask putih — tanpa ubah perilaku default (bubble-only tetap identik).

## Task 1 — OcrEngine regions (trait) — pluggable, tidak fixed rapid + auto-download model
- Extend `src/ocr.rs:5` trait `OcrEngine` dengan `fn recognize(&self, img:&RgbImage, bbox:[u32;4])->Option<String>` (bubble crop) + `fn recognize_regions(&self, full:&RgbImage, exclude:&[[u32;4]])->Vec<TextRegion>` default `Vec::new()`, `struct TextRegion{ bbox:[u32;4], text:String }`
- `NoopOcr` tetap return empty — translate bubble-only tidak berubah. Engine pluggable `match --ocr`: `none`(Noop), `rapid`(RapidOCR ONNX multilingual), `manga`(manga-ocr ONNX JP SOTA), `tesseract`(jpn_vert ringan) — tambah engine baru tinggal `impl OcrEngine` tanpa ubah preparer/mode. Task ini cukup trait+Noop, wiring nanti.
- Model auto-download (link terverifikasi 2026-09-10): pertama `translate --ocr rapid|manga` cek `~/.cache/kzktdk/models/<engine>/` kalau kosong download HF lalu `sha256` verify, simpan permanen next run offline. Tidak commit `.onnx` ke repo — `.gitignore` `models/*.onnx`, opsi `--ocr-model <path>` override lokal.
  - `rapid` (RapidOCR/PaddleOCR ONNX): `SWHL/RapidOCR` `PP-OCRv1/japan_rec_crnn.onnx 3.6MB` + `ch_PP-OCRv3_det 2-5MB` https://huggingface.co/SWHL/RapidOCR , atau `monkt/paddleocr-onnx` `detection/v5/det.onnx 84MB` + `languages/japanese/rec.onnx` https://huggingface.co/monkt/paddleocr-onnx , atau `deepghs/paddleocr` `japan_PP-OCRv3_rec` https://huggingface.co/deepghs/paddleocr — pilih 1 bundle `det+rec+japan dict.txt` `~16MB` via `hf_hub_download`. Alternatif ModelScope `RapidAI/RapidOCR` `onnx/PP-OCRv4/det/ch_PP-OCRv4_det_mobile.onnx` https://github.com/RapidAI/RapidOCR/blob/main/python/rapidocr/default_models.yaml
  - `manga` (SOTA JP manga): `kha-white/manga-ocr-base 444MB torch` https://huggingface.co/kha-white/manga-ocr-base + ONNX `onnx-community/manga-ocr-base-ONNX 1.3GB` https://huggingface.co/onnx-community/manga-ocr-base-ONNX (butuh tokenizer `ViTImageProcessor`+`AutoTokenizer`). Alternatif ringan `liksunrice/manga-ocr-torchless` ONNX Runtime.
  - `tesseract` pakai `tessdata` sistem `apt install tesseract-ocr-jpn jpn_vert` tanpa download HF.
- Dependensi tambahan: `ort 2.x` (onnxruntime 1.18+), `hf-hub`/`reqwest` untuk download+sha256, `image` crate sudah ada, `tokenizers` untuk manga-ocr jika pakai. Fallback offline bila download gagal → log `fallback none`.

## Task 2 — Preparer detect_free_text (port PagePreparer.kt:59)
- Buat `src/preparer.rs` fungsi `detect_free_text(img:&RgbImage, bubble_boxes:&[[u32;4]], ocr:&dyn OcrEngine)->Vec<[u32;4]>`:
  1. Scale `MAX_DETECT_DIM 2048` `scale=min(1,2048/max(w,h))`, clone image, fill `WHITE` tiap `excludeBoxes` (mask bubble) sebelum resize — port `LocalOcrEngine.kt:172 canvas.drawRect WHITE`
  2. Panggil `ocr.recognize_regions(scaled, &[])` → scale balik `*1/scale` ke koordinat asli
  3. Filter: `w<12||h<8` skip, `trim empty` skip, `len==1 [A-Za-z0-9/symbol]` skip `PagePreparer.kt:73`, `center inside bubble` skip `85`, `rectIou>=0.3` skip `89`
  4. `merge_nearby_text_boxes` cluster gap `20px` port `ImageProcessor.mergeNearbyTextBoxes:98`
- Fungsi `crop_free_text` + `crop_bubbles` helper: pad `6 / 6%` freetext, `padXRatio/padYRatio` bubble, ID `ft1..` / `"<prefix>ftN"` tidak tabrakan `1,2`, bg median sampling clean style.

## Task 3 — CLI flags (port OcrScript + translateFreeText + mode Vision vs OCR)
- `translate` dan `detect` tambah `--translate-free-text` bool default `false` + `--ocr-script <auto|jp|en|kr|cn>` default `jp` `OcrScript.fromKey` `LocalOcrEngine.kt:28` (JAPANESE+Latin). `--ocr` sudah ada `none|rapid|manga|tesseract` — bebas ganti, tidak fixed rapid; `auto` untuk rapid union semua rec model.
- Tambah `--mode <vision|ocr|auto>` default `vision` untuk bedakan jalur: `vision`=kirim mosaic `translateImage` (sekarang), `ocr`=OCR `crop→rawText JSON` → `translateText` text-only tanpa gambar (murah, `ChunkTranslator.kt:168 translateOcrChunk`), `auto`=coba OCR dulu fallback vision bila `ocrMap empty`/gagal. `--ocr` saja tanpa `--mode ocr` tetap vision (hanya isi `raw_text` untuk `show --show-raw`), harus eksplisit `--mode ocr` baru pakai text path.
- Jika `--translate-free-text` tanpa engine `rapid/manga/tesseract` (`none`) → `eprintln!("[freetext] butuh --ocr rapid|manga|tesseract, fallback bubble-only")` dan jangan panggil `detect_free_text`.

## Task 4 — Pipeline wiring + metadata IDs (Vision vs OCR)
- `translate` flow `src/main.rs` setelah `yolo.detect_bubbles` → jika `translate_free_text` panggil `detect_free_text`, `crop_free_text`, gabung `crops = [bubbles, ft]` ke mosaic/metadata dengan `Bubble{id="ft1", bbox, raw_text}`. `detect` export juga ikut.
- Branch `mode`: `vision` → `ChunkTranslator.translateVisionChunk`-style `buildMosaic → translateImage`; `ocr` → per crop `ocr.recognize` → `ocrMap` → `translateText(json)` text-only (`dummy 1x1` fallback) `ChunkTranslator.kt:206`; `auto` → coba ocr dulu fallback vision. Freetext ikut di kedua mode (butuh ocr untuk deteksi).
- `metadata show/render/preview/edit/pack` otomatis handle `ft` IDs (`FT_KEY_REGEX (\d+_)?ft\d+` `ChunkTranslator.kt:311`) — tidak perlu flag baru, `show` kolom `RAW` sudah ada.

## Verifikasi
- `cargo build` OK, `translate page.jpg --save-metadata -o out/` tanpa flag → hanya `1,2,3` (vision bubble-only backward compat)
- `translate page.jpg --ocr rapid --mode ocr --save-metadata -o out/` → jalur text-only tanpa mosaic, `show` `raw_text` terisi
- `translate page.jpg --ocr rapid --translate-free-text --save-metadata -o out/ && metadata show out/page.kedit.json | grep ft` → `ft1 ft2` muncul (vision mode), `preview --id ft1 --show-raw` md5 beda, `edit --set ft1=Baru` + `render` success
- `translate page.jpg --ocr tesseract --mode ocr --translate-free-text` + `--ocr-script auto` → engine ganti tetap jalan
- Tanpa engine (`--ocr none`) + `--translate-free-text` atau `--mode ocr` → log fallback vision/bubble-only, hanya bubble
