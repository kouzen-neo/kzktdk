//! Tauri batch-translate binding tests (require `--features tauri`).
//!
//! Without the feature this file compiles to nothing and `cargo test`
//! stays green. Config-validation tests need no model and no network.
//! The end-to-end failure-path test needs `models/kzkt.onnx` (decrypted
//! locally, not committed) and skips loudly when absent; it uses a
//! closed-port LLM endpoint so no network is touched.

#![cfg(feature = "tauri")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use kzktdk::tauri::{PageResult, ProgressEvent, TranslateConfig, editor_translate_batch};

fn closed_port_config(model: &str) -> TranslateConfig {
    TranslateConfig {
        provider: "ollama".to_string(),
        // Closed port: connection refused fast, no network involved.
        openai_base_url: "http://127.0.0.1:9/v1".to_string(),
        openai_model: "nope".to_string(),
        rate_limit: 10,
        batch_size: 2,
        model: model.to_string(),
        use_cache: false,
        save_metadata: false,
        ..Default::default()
    }
}

#[test]
fn config_defaults_are_sane() {
    let c = TranslateConfig::default();
    assert_eq!(c.target_lang, "English");
    assert_eq!(c.provider, "gemini");
    assert_eq!(c.mode, "vision");
    assert!(c.use_cache);
    let ev = ProgressEvent {
        idx: 1,
        total: 1,
        page: "p.png".to_string(),
        phase: "done",
        error: None,
    };
    assert_eq!(ev.phase, "done");
    assert!(ev.error.is_none());
}

#[test]
fn empty_input_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let seen: Arc<Mutex<Vec<ProgressEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_c = seen.clone();
    let r = editor_translate_batch(
        vec![],
        dir.path().to_str().unwrap(),
        TranslateConfig::default(),
        move |ev| {
            seen_c.lock().unwrap().push(ev);
        },
    );
    assert!(r.is_err());
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn missing_image_is_rejected_before_any_work() {
    let dir = tempfile::tempdir().unwrap();
    let r = editor_translate_batch(
        vec!["/definitely/not/here.png".to_string()],
        dir.path().to_str().unwrap(),
        TranslateConfig::default(),
        |_| {},
    );
    let e = r.unwrap_err();
    assert!(e.contains("not found"), "unexpected: {e}");
}

#[test]
fn unknown_provider_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("p.png");
    image::RgbImage::from_pixel(64, 64, image::Rgb([255, 255, 255]))
        .save(&png)
        .unwrap();
    let mut c = TranslateConfig::default();
    c.provider = "watson".to_string();
    let r = editor_translate_batch(
        vec![png.to_str().unwrap().to_string()],
        dir.path().join("out").to_str().unwrap(),
        c,
        |_| {},
    );
    let e = r.unwrap_err();
    assert!(e.contains("Unknown provider"), "unexpected: {e}");
}

#[test]
fn invalid_glossary_is_rejected_before_model_load() {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("p.png");
    image::RgbImage::from_pixel(64, 64, image::Rgb([255, 255, 255]))
        .save(&png)
        .unwrap();
    let gloss = dir.path().join("bad.json");
    std::fs::write(&gloss, r#"{"Luffy": ""}"#).unwrap();
    // ollama needs no key; model path bogus to prove glossary wins.
    let mut c = closed_port_config("/definitely/not/a/model.onnx");
    c.glossary = Some(gloss.to_str().unwrap().to_string());
    let r = editor_translate_batch(
        vec![png.to_str().unwrap().to_string()],
        dir.path().join("out").to_str().unwrap(),
        c,
        |_| {},
    );
    let e = r.unwrap_err();
    assert!(e.to_lowercase().contains("glossary"), "unexpected: {e}");
}

#[test]
fn missing_model_file_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("p.png");
    image::RgbImage::from_pixel(64, 64, image::Rgb([255, 255, 255]))
        .save(&png)
        .unwrap();
    let c = closed_port_config("/definitely/not/a/model.onnx");
    let r = editor_translate_batch(
        vec![png.to_str().unwrap().to_string()],
        dir.path().join("out").to_str().unwrap(),
        c,
        |_| {},
    );
    assert!(r.is_err());
}

#[test]
fn failed_page_emits_detect_end_then_failed() {
    let model = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models/kzkt.onnx");
    if !model.is_file() {
        eprintln!("SKIP failed_page_emits_detect_end_then_failed: models/kzkt.onnx absent");
        return;
    }
    // Real manga-ish page (local fixture, not committed) so YOLO finds a
    // bubble and the pipeline reaches the (doomed) LLM call.
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_inpainted.png");
    if !fixture.is_file() {
        eprintln!("SKIP failed_page_emits_detect_end_then_failed: test fixture absent");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("p.png");
    std::fs::copy(&fixture, &png).unwrap();

    let out = dir.path().join("out");
    let seen: Arc<Mutex<Vec<ProgressEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_c = seen.clone();
    let c = closed_port_config(model.to_str().unwrap());
    let results: Vec<PageResult> = editor_translate_batch(
        vec![png.to_str().unwrap().to_string()],
        out.to_str().unwrap(),
        c,
        move |ev| {
            seen_c.lock().unwrap().push(ev);
        },
    )
    .expect("setup should succeed; only the LLM call fails");
    assert_eq!(results.len(), 1);
    assert!(!results[0].ok);
    assert!(results[0].error.is_some());
    // Original copied on failure (CLI parity).
    assert!(PathBuf::from(&results[0].output).is_file());

    let evs = seen.lock().unwrap();
    let phases: Vec<&str> = evs.iter().map(|e| e.phase).collect();
    assert!(phases.contains(&"detect_end"), "phases seen: {phases:?}");
    let last = evs.last().unwrap();
    assert_eq!(last.phase, "failed");
    assert_eq!(last.idx, 1);
    assert_eq!(last.total, 1);
    assert!(last.error.is_some());
}

#[test]
fn hostile_inputs_fail_gracefully_never_panic() {
    // Fase-3 anti-panic gauntlet: corrupt/empty images must surface as
    // per-page `failed` (original copied when possible), and setup-level
    // garbage (dir-as-image, file-as-out-dir) as `Err` — never a panic.
    // Completing this test IS the assertion; any panic fails it.
    let model = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models/kzkt.onnx");
    if !model.is_file() {
        eprintln!("SKIP hostile_inputs_fail_gracefully_never_panic: models/kzkt.onnx absent");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let corrupt = dir.path().join("corrupt.png");
    std::fs::write(&corrupt, vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x89, 0x50]).unwrap();
    let empty = dir.path().join("empty.png");
    std::fs::write(&empty, b"").unwrap();

    let out = dir.path().join("out");
    let seen: Arc<Mutex<Vec<ProgressEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_c = seen.clone();
    let c = closed_port_config(model.to_str().unwrap());
    let results: Vec<PageResult> = editor_translate_batch(
        vec![
            corrupt.to_str().unwrap().to_string(),
            empty.to_str().unwrap().to_string(),
        ],
        out.to_str().unwrap(),
        c,
        move |ev| {
            seen_c.lock().unwrap().push(ev);
        },
    )
    .expect("hostile pages must be per-page failures, not setup errors");
    assert_eq!(results.len(), 2);
    for r in &results {
        assert!(!r.ok, "hostile input unexpectedly ok: {r:?}");
        assert!(r.error.is_some(), "missing error detail: {r:?}");
    }
    let failed: Vec<_> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.phase == "failed")
        .map(|e| e.idx)
        .collect();
    assert_eq!(failed, vec![1, 2], "expected one failed event per page");

    // Setup-level garbage: directory as image, regular file as out dir.
    let subdir = dir.path().join("sub");
    std::fs::create_dir_all(&subdir).unwrap();
    let r = editor_translate_batch(
        vec![subdir.to_str().unwrap().to_string()],
        out.to_str().unwrap(),
        closed_port_config(model.to_str().unwrap()),
        |_| {},
    );
    assert!(r.is_err(), "directory input should be rejected");

    let file_as_out = dir.path().join("file_out");
    std::fs::write(&file_as_out, b"x").unwrap();
    let lone = dir.path().join("lone.png");
    image::RgbImage::from_pixel(8, 8, image::Rgb([255, 255, 255]))
        .save(&lone)
        .unwrap();
    let r = editor_translate_batch(
        vec![lone.to_str().unwrap().to_string()],
        file_as_out.to_str().unwrap(),
        closed_port_config(model.to_str().unwrap()),
        |_| {},
    );
    assert!(r.is_err(), "file-as-out-dir should be rejected");
}

#[test]
fn stub_ocr_rec_is_rejected_before_model_load() {
    // Fase-4: freetext/mode-ocr with a stub engine is a setup Err
    // (hermetic: 0-byte image passes the is_file check, guard fires first).
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("p.png");
    std::fs::write(&png, b"").unwrap();
    for (freetext, mode) in [(true, "vision"), (false, "ocr")] {
        let mut c = TranslateConfig::default();
        c.ocr = "rapid".to_string();
        c.translate_free_text = freetext;
        c.mode = mode.to_string();
        let r = editor_translate_batch(
            vec![png.to_str().unwrap().to_string()],
            dir.path().join("out").to_str().unwrap(),
            c,
            |_| {},
        );
        let e = r.unwrap_err();
        assert!(
            e.contains("not implemented"),
            "freetext={freetext} mode={mode}: unexpected: {e}"
        );
    }
}
