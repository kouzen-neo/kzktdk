# GPU Acceleration Roadmap (ONNX Runtime EPs)

Status: **research only** — inference stays CPU-default. No code changed.

Reference: `ort` 2.0 docs (`perf/execution-providers`), crate in use:
`ort = "2.0.0-rc.13"`.

## How enabling works (when we do it)

1. Add a cargo feature per execution provider (EP):

   ```toml
   [dependencies]
   ort = { version = "2.0", features = ["cuda"] }
   ```

2. Register EPs in priority order in `YoloModel::new`
   (`src/model/yolo.rs`, today `Session::builder()?.commit_from_file(path)?`):

   ```rust
   use ort::{ep, session::Session};

   let session = Session::builder()?
       .with_execution_providers([
           #[cfg(feature = "tensorrt")]
           ep::TensorRT::default().build(),   // prefer TRT over CUDA
           #[cfg(feature = "cuda")]
           ep::CUDA::default().build(),
           #[cfg(feature = "directml")]
           ep::DirectML::default().build(),   // Windows fallback
           #[cfg(feature = "coreml")]
           ep::CoreML::default()
               .with_compute_units(ep::coreml::ComputeUnits::CPUAndNeuralEngine)
               .build(),                       // Apple ANE
       ])?
       .commit_from_file(model_path)?;
   ```

   The runtime falls back to CPU automatically for unsupported ops
   (unless `.error_on_failure()` is set — do NOT set it; CPU fallback
   must keep working everywhere).

3. Gate everything behind cfg + a `--device cpu|cuda|directml|coreml|auto`
   CLI flag (default `cpu` until proven stable), and surface the active EP
   in `--progress jsonl` / GUI batch events.

## Per-OS plan

| OS      | EP (priority)              | Cargo feature | System dependency                              |
|---------|----------------------------|---------------|------------------------------------------------|
| Linux   | CUDA → TensorRT (opt-in)   | `cuda`, `tensorrt` | NVIDIA driver + CUDA toolkit (+ cuDNN / TRT) |
| Windows | CUDA → DirectML            | `cuda`, `directml` | NVIDIA driver; DirectML ships with Windows 10+ |
| macOS   | CoreML (ANE)               | `coreml`      | System frameworks only (no extra install)      |

CPU (`XNNPACK`/`oneDNN` via default features) remains the baseline and the
CI-tested path.

## Open questions before implementation

- Binary size + build time per EP feature (measure before enabling by default).
- YOLO model used here is small (640px); measure real detect latency
  CPU vs GPU on target hardware — GPU may only pay off in batch/GUI use.
- `YoloModel` is `Send` (proven, see `send_tests`); EP sessions keep that
  property, but re-verify with the `assert_send` tests after switching.
- OCR engines (`src/ocr.rs`) also build `ort::Session` — same treatment.
- GUI installers must document/locate EP system libs (same class of problem
  as `docs/PDFIUM.md`).

See also: `README.md` (Contributing & GUI Roadmap), `docs/UPDATE_ROADMAP.md`.
