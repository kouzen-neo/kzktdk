# KZKT-DK Technical Architecture & Specifications

This document details the internal design, mathematical formulations, and engineering decisions behind `kzktdk`.

---

## 1. System Overview

`kzktdk` is a high-performance native Rust translation pipeline for comic and manga imagery. It replicates the core processing algorithms of [KZKT Android](https://github.com/kouzen-neo/kzkt) without relying on Python or OpenCV C++ libraries.

```text
               Input Image / Folder / CBZ
                           │
                           ▼
          ┌──────────────────────────────────┐
          │  YOLOv8 Speech Bubble Detector   │ (ONNX Runtime)
          └──────────────────────────────────┘
                           │
                 [Detections: BBoxes]
                           │
             ┌─────────────┴─────────────┐
             ▼                           ▼
    ┌─────────────────┐         ┌─────────────────┐
    │  Mosaic Builder │         │  Interior Mask  │
    └─────────────────┘         │  & Dilation     │
             │                  └─────────────────┘
    [Vertical Mosaic]                    │
             │                           ▼
             ▼                  ┌─────────────────┐
    ┌─────────────────┐         │  Telea Inpaint  │
    │  LLM Translator │         │  (Pure Rust)    │
    └─────────────────┘         └─────────────────┘
             │                           │
    [Text Translations]          [Cleaned Image]
             │                           │
             └─────────────┬─────────────┘
                           ▼
          ┌──────────────────────────────────┐
          │   Diamond / Elliptical Typesetter│
          │   & Phonetic Syllable Wrap       │
          └──────────────────────────────────┘
                           │
                           ▼
              Translated Output (Image/CBZ)
```

---

## 2. Speech Bubble Detection (YOLOv8 Cascade)

### 2.1 Inference Pipeline
- Uses Microsoft ONNX Runtime (`ort` v2.0) with multi-threaded CPU execution.
- Input: Image resized to `640x640` with letterbox padding.
- Normalization: RGB values normalized to $[0.0, 1.0]$.
- Tensor shape: `[1, 3, 640, 640]` in `NCHW` planar order.

### 2.2 3-Stage Confidence Cascade
To capture faint sketched bubbles without introducing false positives, a three-stage cascade filter is employed:

$$\begin{aligned}
\text{Stage 1 (Primary):} \quad & \text{Confidence} \ge 0.28, \; \text{IoU Threshold} = 0.45 \\
\text{Stage 2 (Secondary):} \quad & \text{Confidence} \ge 0.18, \; \text{IoU Threshold} = 0.55 \\
\text{Stage 3 (Sensitive):} \quad & \text{Confidence} \ge 0.10, \; \text{IoU Threshold} = 0.65
\end{aligned}$$

### 2.3 Geometric False-Positive Filtering
- **Aspect Ratio Guard**: Eliminates bounding boxes where width-to-height or height-to-width ratio exceeds $12:1$.
- **False-Giant Suppression**: Discards any detected box exceeding $92\%$ of total page area, preventing full-page false detections from suppressing legitimate interior bubbles.

---

## 3. Mask Generation & Telea Fast Marching Inpainting

### 3.1 Bubble Interior Isolation
To prevent destroying black manga speech bubble borders or background screentones, text extraction is constrained to the interior region:
1. **Luminance Calculation**:
   $$\text{Luminance}(R, G, B) = 0.299R + 0.587G + 0.114B$$
2. **Background Classification**:
   - Median background luminance inside the bubble determines whether the bubble is light (standard dialogue) or dark (screentone / inverted).
3. **Contrast Masking**:
   - For light bubbles, pixels darker than the background threshold by a margin $\Delta L \ge 45$ are marked as text strokes.
   - For dark bubbles, pixels significantly lighter than the background are marked.

### 3.2 Anti-Aliasing & Stroke Dilation
Text rasterization creates semi-transparent anti-aliased edge pixels. If left unmasked, these leave noticeable gray halos after inpainting.
- A circular structuring element of radius $r = 2$ dilates the text stroke mask.
- Morphological dilation ensures all sub-pixel fringe artifacts are enclosed within the inpainting domain.

### 3.3 Telea Fast Marching Method (Pure Rust)
The inpainting engine uses the Alexandru Telea algorithm implemented in pure Rust (`inpaint` crate):
- Computes the Eikonal equation using Fast Marching Method (FMM) to propagate color gradients from the boundary inward.
- Preserves paper texture, subtle gradients, and background screentones seamlessly without blur or discoloration.

---

## 4. Diamond & Elliptical Typesetting Engine

Speech bubbles in comics and manga are primarily oval or elliptical. Standard rectangular wrapping creates ugly line breaks or overflows at the top and bottom corners.

### 4.1 Elliptical Width Function
For a bubble of usable width $W_{\max}$ and total lines $N$:
$$\hat{y}_i = \frac{2i + 1}{N} - 1, \quad \hat{y}_i \in [-1, 1]$$
$$\text{Scale}(i) = \max\left(0.82, \; \sqrt{\max\left(0.20, \; 1.0 - 0.40 \cdot \hat{y}_i^2\right)}\right)$$
$$W_i = W_{\max} \cdot \text{Scale}(i)$$

This creates a diamond/elliptical bounding contour that naturally fills the center of the bubble wider and tapers the top and bottom lines.

### 4.2 Phonetic Syllable Hyphenator
To prevent awkward breaks (e.g. `d-` on one line and `iatas` on the next), the layout engine incorporates an Indonesian and Latin phonetic syllabification module:
- Implements standard phonotactic and EYD syllabification rules.
- Identifies consonant clusters (`ng`, `ny`, `sy`, `kh`), diphthongs (`ai`, `au`, `oi`), and syllable boundaries ($V$-$CV$, $VC$-$CV$, $V$-$V$).
- Generates valid hyphenation candidates when long words exceed line limits.

---

## 5. Model Decryption Specification

The bundled model (`models/kzkt.dat`) is an obfuscated binary from the KZKT distribution.

- **Algorithm**: Byte-wise symmetric stream XOR with key `0x5A` (`90` decimal).
- **Validation**:
  - The first byte of an ONNX Protobuf payload must match `0x08` (Protobuf tag 1 for `ir_version`).
  - Decrypted header is verified before writing the full model to `models/kzkt.onnx`.
