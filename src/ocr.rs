use image::RgbImage;

/// Abstraction for future OCR engines (tesseract / vision).
/// Current impl is Noop — translate pipeline stays Vision-only.
pub trait OcrEngine: Send + Sync {
    fn recognize(&self, _image: &RgbImage, _bbox: [u32; 4]) -> Option<String> {
        None
    }
    fn name(&self) -> &'static str {
        "none"
    }
}

pub struct NoopOcr;
impl OcrEngine for NoopOcr {
    fn recognize(&self, _image: &RgbImage, _bbox: [u32; 4]) -> Option<String> {
        None
    }
    fn name(&self) -> &'static str {
        "none"
    }
}
