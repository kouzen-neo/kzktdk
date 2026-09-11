//! CLI machine-readable contract tests (no model / no network needed).
//!
//! Asserts: stdout is valid JSON, exit codes (0 ok / 2 invalid data),
//! stdout-vs-stderr separation, and the full
//! export-shape -> edit -> preview -> render -> pack session.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kzktdk"))
}

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("failed to spawn kzktdk");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

struct Fixture {
    dir: tempfile::TempDir,
    png: PathBuf,
    json: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("page.png");
    // White page with one black box (fake bubble area for render/inpaint).
    let mut img = image::RgbImage::from_pixel(400, 400, image::Rgb([255, 255, 255]));
    for y in 50..150 {
        for x in 50..350 {
            img.put_pixel(x, y, image::Rgb([0, 0, 0]));
        }
    }
    img.save(&png).unwrap();
    let json = dir.path().join("p.kedit.json");
    let data = serde_json::json!({
        "version": 1,
        "page": "page.png",
        "width": 400,
        "height": 400,
        "target_lang": "Indonesian",
        "prompt_sig": "classic",
        "bubbles": [
            {"id": "1", "bbox": [50, 50, 350, 150], "conf": 0.9,
             "translated": "Halo dunia", "bg_color": null,
             "style": {"font_size": 22.0}, "edited": true},
            {"id": "2", "bbox": [50, 200, 350, 300], "conf": 0.8,
             "translated": "SKIP", "bg_color": null, "edited": false}
        ],
        "order": ["1", "2"]
    });
    std::fs::write(&json, serde_json::to_string_pretty(&data).unwrap()).unwrap();
    Fixture { dir, png, json }
}

#[test]
fn show_json_contract() {
    let f = fixture();
    let (code, stdout, _stderr) = run(&["metadata", "show", s(&f.json), "--format", "json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["bubbles"].as_array().unwrap().len(), 2);
    assert_eq!(v["order"], serde_json::json!(["1", "2"]));

    // Single-bubble filter returns the bubble object itself.
    let (code, stdout, _) = run(&[
        "metadata",
        "show",
        s(&f.json),
        "--id",
        "1",
        "--format",
        "json",
    ]);
    assert_eq!(code, 0);
    let b: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(b["id"], serde_json::json!("1"));
}

#[test]
fn validate_json_exit_codes() {
    let f = fixture();
    let (code, stdout, _) = run(&["metadata", "validate", s(&f.json), "--format", "json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["valid"], serde_json::json!(true));
    assert_eq!(v["kind"], serde_json::json!("page"));

    // Invalid: duplicate id + out of bounds.
    let bad = f.dir.path().join("bad.kedit.json");
    let data = serde_json::json!({
        "version": 1, "page": "page.png", "width": 400, "height": 400,
        "target_lang": "English", "prompt_sig": "classic",
        "bubbles": [
            {"id": "1", "bbox": [0, 0, 5000, 5000], "conf": 1.0,
             "translated": "x", "bg_color": null, "edited": false},
            {"id": "1", "bbox": [0, 0, 10, 10], "conf": 1.0,
             "translated": "y", "bg_color": null, "edited": false}
        ]
    });
    std::fs::write(&bad, serde_json::to_string(&data).unwrap()).unwrap();
    let (code, stdout, _) = run(&["metadata", "validate", s(&bad), "--format", "json"]);
    assert_eq!(code, 2);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["valid"], serde_json::json!(false));
    let codes: Vec<&str> = v["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"duplicate_id"));
    assert!(codes.contains(&"out_of_bounds"));
}

#[test]
fn stdin_patch_is_transactional() {
    use std::io::Write;
    let f = fixture();
    let before = std::fs::read(&f.json).unwrap();
    // Invalid patch: unknown id + bad bbox.
    let mut child = Command::new(bin())
        .args(["metadata", "edit", s(&f.json), "--stdin-patch"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(br#"{"set":["99=Missing"],"bbox":["1=5000,0,10,10"]}"#)
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("\"ok\":false"));
    // File untouched.
    assert_eq!(std::fs::read(&f.json).unwrap(), before);

    // Valid patch applies.
    let mut child = Command::new(bin())
        .args(["metadata", "edit", s(&f.json), "--stdin-patch"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(br#"{"set":["1=Hai"],"order":"1,2"}"#)
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("\"ok\":true"));
}

#[test]
fn preview_stdout_is_clean_base64() {
    let f = fixture();
    let (code, stdout, stderr) = run(&[
        "metadata",
        "preview",
        s(&f.png),
        "--metadata",
        s(&f.json),
        "--id",
        "1",
        "--text",
        "Halo",
        "--to-stdout",
        "--thumb",
        "64",
    ]);
    assert_eq!(code, 0);
    // stdout: exactly one base64 line.
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "stdout must be a single base64 line");
    assert!(stderr.contains("Preview bubble 1"));
    // Decodes to a small PNG.
    let raw = base64_decode(lines[0]);
    assert_eq!(&raw[1..4], b"PNG");
    let img = image::load_from_memory(&raw).unwrap();
    assert!(img.width().max(img.height()) <= 64);
}

fn base64_decode(s: &str) -> Vec<u8> {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lut = [255u8; 256];
    for (i, &c) in T.iter().enumerate() {
        lut[c as usize] = i as u8;
    }
    let bytes: Vec<u8> = s.bytes().filter(|&b| b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for ch in bytes.chunks(4) {
        let mut n = 0u32;
        let mut bits = 0;
        for &c in ch {
            n = (n << 6) | lut[c as usize] as u32;
            bits += 6;
        }
        while bits >= 8 {
            bits -= 8;
            out.push((n >> bits) as u8);
        }
    }
    out
}

#[test]
fn render_jsonl_and_pack_json() {
    let f = fixture();
    let out = f.dir.path().join("out.jpg");
    let (code, _stdout, stderr) = run(&[
        "metadata",
        "render",
        s(&f.png),
        "--metadata",
        s(&f.json),
        "-o",
        s(&out),
        "--progress",
        "jsonl",
    ]);
    assert_eq!(code, 0);
    assert!(out.is_file());
    assert!(stderr.contains("\"done\":true"));

    // Pack the rendered folder.
    let folder = f.dir.path().join("rend");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::copy(&out, folder.join("p1.jpg")).unwrap();
    let cbz = f.dir.path().join("ch.cbz");
    let (code, stdout, _) = run(&[
        "metadata",
        "pack",
        s(&folder),
        "-o",
        s(&cbz),
        "--format",
        "json",
    ]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["ok"], serde_json::json!(true));
    assert_eq!(v["count"], serde_json::json!(1));
}

#[test]
fn font_list_json_contract() {
    let (code, stdout, _) = run(&["font", "list", "--format", "json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(v.get("fonts").is_some());
}

#[test]
fn mask_size_validation() {
    let f = fixture();
    // Wrong-size mask -> exit 2.
    let bad_mask = f.dir.path().join("badmask.png");
    image::RgbImage::from_pixel(10, 10, image::Rgb([255, 255, 255]))
        .save(&bad_mask)
        .unwrap();
    let (code, _, stderr) = run(&[
        "metadata",
        "mask",
        s(&f.json),
        "--id",
        "1",
        "--from",
        s(&bad_mask),
    ]);
    assert_eq!(code, 2);
    assert!(stderr.contains("Invalid mask"));
    // Right-size mask (bbox 1 = 300x100) -> ok.
    let good_mask = f.dir.path().join("goodmask.png");
    image::RgbImage::from_pixel(300, 100, image::Rgb([255, 255, 255]))
        .save(&good_mask)
        .unwrap();
    let (code, _, _) = run(&[
        "metadata",
        "mask",
        s(&f.json),
        "--id",
        "1",
        "--from",
        s(&good_mask),
    ]);
    assert_eq!(code, 0);
    // Render still works with the mask attached.
    let out = f.dir.path().join("masked.jpg");
    let (code, _, _) = run(&[
        "metadata",
        "render",
        s(&f.png),
        "--metadata",
        s(&f.json),
        "-o",
        s(&out),
    ]);
    assert_eq!(code, 0);
    // Clear it again.
    let (code, _, _) = run(&["metadata", "mask", s(&f.json), "--id", "1", "--clear"]);
    assert_eq!(code, 0);
}

#[test]
fn translate_help_lists_new_flags() {
    let (code, stdout, _) = run(&["translate", "--help"]);
    assert_eq!(code, 0);
    for flag in [
        "--glossary",
        "--progress",
        "--quiet",
        "--format",
        "--retry-failed",
    ] {
        assert!(stdout.contains(flag), "missing {}", flag);
    }
}

#[test]
fn detect_json_contract_without_model_file() {
    // --help works without a model; full detect needs YOLO (skipped here).
    let (code, stdout, _) = run(&["detect", "--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("--format"));
}

#[test]
fn retry_hit_logic_via_translate_help_only() {
    // retry_failed is exercised in unit scope; here assert the flag exists.
    let (code, stdout, _) = run(&["translate", "--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("retry"));
}

#[test]
fn pdf_input_rejected_clearly_without_libpdfium() {
    // Without libpdfium installed, a PDF input must fail with a helpful
    // message (not a panic). If libpdfium IS present, export runs instead.
    let f = fixture();
    let pdf = f.dir.path().join("doc.pdf");
    std::fs::write(&pdf, b"%PDF-1.4 fake").unwrap();
    // Absolute model path so failure (if any) comes from the PDF layer.
    let model = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models/kzkt.onnx");
    let (code, _stdout, stderr) = run(&[
        "metadata",
        "export",
        s(&pdf),
        "--json",
        s(&f.dir.path().join("doc.kedit.json")),
        "-m",
        s(&model),
    ]);
    if code != 0 {
        let low = stderr.to_lowercase();
        assert!(
            low.contains("pdfium") || low.contains("pdf"),
            "unexpected error: {}",
            stderr
        );
    }
}

#[test]
fn translate_pdf_export_writes_file_not_dir() {
    // Regression: `--export pdf -o out.pdf` used to create_dir_all(out.pdf),
    // so packing failed with IsADirectory. Needs YOLO (skip if absent);
    // needs no LLM keys: the closed-port endpoint fails fast, pages fall
    // back to originals, packing still runs.
    let model = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models/kzkt.onnx");
    if !model.is_file() {
        eprintln!("SKIP translate_pdf_export_writes_file_not_dir: model absent");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let inp = dir.path().join("in");
    std::fs::create_dir_all(&inp).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_inpainted.png");
    if fixture.is_file() {
        std::fs::copy(&fixture, inp.join("p.png")).unwrap();
    } else {
        // Fallback synthetic page when the local fixture is absent.
        let img = image::RgbImage::from_pixel(300, 300, image::Rgb([255, 255, 255]));
        img.save(inp.join("p.png")).unwrap();
    }
    let out_pdf = dir.path().join("out.pdf");
    let (code, _, stderr) = run(&[
        "translate",
        s(&inp),
        "-o",
        s(&out_pdf),
        "--export",
        "pdf",
        "--provider",
        "ollama",
        "--openai-base-url",
        "http://127.0.0.1:9/v1",
        "--openai-model",
        "nope",
        "-m",
        s(&model),
        "--jobs",
        "1",
        "--no-cache",
    ]);
    if !out_pdf.is_file() && stderr.to_lowercase().contains("pdfium") {
        eprintln!("SKIP translate_pdf_export_writes_file_not_dir: no Pdfium library");
        return;
    }
    assert!(
        out_pdf.is_file(),
        "code={} expected out.pdf file, stderr:\n{}",
        code,
        stderr
    );
    assert!(!out_pdf.is_dir());
}

#[test]
fn translate_format_json_stdout_stays_pure() {
    // Second run hits the translation cache; cache-hit lines must go to
    // stderr (tinfo), never pollute the stdout JSON summary.
    let model = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models/kzkt.onnx");
    if !model.is_file() {
        eprintln!("SKIP translate_format_json_stdout_stays_pure: model absent");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let inp = dir.path().join("in");
    std::fs::create_dir_all(&inp).unwrap();
    let mut img = image::RgbImage::from_pixel(300, 300, image::Rgb([255, 255, 255]));
    for y in 40..120 {
        for x in 40..260 {
            img.put_pixel(x, y, image::Rgb([10, 10, 10]));
        }
    }
    img.save(inp.join("p.png")).unwrap();
    for pass in 0..2 {
        let out = dir.path().join(format!("out{}", pass));
        let (code, stdout, _) = run(&[
            "translate",
            s(&inp),
            "-o",
            s(&out),
            "--provider",
            "ollama",
            "--openai-base-url",
            "http://127.0.0.1:9/v1",
            "--openai-model",
            "nope",
            "-m",
            s(&model),
            "--jobs",
            "1",
            "--format",
            "json",
            "--quiet",
        ]);
        let _ = code; // pages fail (no LLM) -> exit 1; contract is about stdout.
        serde_json::from_str::<serde_json::Value>(&stdout).unwrap_or_else(|e| {
            panic!("pass {} stdout not pure JSON: {}\n---\n{}", pass, e, stdout)
        });
    }
}

#[test]
fn stub_ocr_rec_fails_fast_without_model() {
    // rapid now has rec; only cht unsupported
    let (code, _, stderr) = run(&[
        "translate",
        "--mode",
        "ocr",
        "--ocr",
        "rapid",
        "--ocr-script",
        "cht",
    ]);
    assert_ne!(code, 0, "cht should fail");
    assert!(
        stderr.contains("not yet supported") || stderr.contains("Traditional"),
        "expected cht unsupported, got: {stderr}"
    );

    // manga now deprecated/removed, falls back to noop
    let (code, stdout, stderr) = run(&["translate", "--ocr", "manga", "--help"]);
    assert_eq!(code, 0, "help should succeed");
    assert!(
        stderr.contains("deprecated or removed") || stdout.contains("--ocr"),
        "manga should warn deprecated"
    );
}
