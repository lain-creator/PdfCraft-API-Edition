//! Read-only, permission-checked preparation for API translation. No history/save changes.
use crate::{DocId, Document, Session};
pub use pdfcraft_ai::translation::{Direction, Source, Translation};
use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RequestKind, Tile};
use std::sync::Arc;

impl Document {
    /// Authorize the exact UI selection before it may leave the application.
    pub fn translation_text(&self, text: String) -> Result<Source, String> {
        self.check_copying().map_err(|e| e.to_string())?;
        pdfcraft_ai::translation::validate_text(&text)?;
        Ok(Source::Text(text))
    }
}
#[derive(Clone)]
pub struct ImageSelection {
    bytes: Arc<Vec<u8>>,
    config: RenderConfig,
    page: usize,
    rect: [f32; 4],
    size: (f32, f32),
    rotation: u16,
}
impl Session {
    /// rect is in the renderer's displayed-page coordinates, before transient view rotation.
    pub fn translation_image(&self, id: DocId, page: usize, rect: [f32; 4], rotation: u16) -> Result<ImageSelection, String> {
        let doc = self.get(id).ok_or("文档已关闭。")?;
        doc.check_copying().map_err(|e| e.to_string())?;
        let info = doc.info.pages.get(page).ok_or("框选页面不存在。")?;
        let size = (info.width, info.height);
        crop_tile(rect, size)?;
        if !matches!(rotation, 0 | 90 | 180 | 270) {
            return Err("页面旋转角度无效。".into());
        }
        Ok(ImageSelection { bytes: doc.bytes.clone(), config: self.render_config(id).ok_or("文档不可读取。")?, page, rect, size, rotation })
    }
    /// Headless counterpart of a contiguous text selection, using zero-based glyph indexes.
    pub fn translation_glyphs(&self, id: DocId, page: usize, start: usize, end: usize) -> Result<Source, String> {
        let doc = self.get(id).ok_or("文档已关闭。")?;
        doc.check_copying().map_err(|e| e.to_string())?;
        if page >= doc.info.pages.len() {
            return Err("页码超出范围。".into());
        }
        crate::guard(|| {
            let mut renderer = PageRenderer::new(doc.bytes.clone(), self.render_config(id).ok_or("文档不可读取。")?);
            let out = renderer.render(RenderRequest { page, kind: RequestKind::Text, ..Default::default() });
            if let Some(e) = out.error {
                return Err(e);
            }
            let text = out.text.ok_or("页面没有可选文字，请使用图片框选。")?;
            if start >= end || end > text.glyphs.len() {
                return Err("文字选区超出范围。".into());
            }
            doc.translation_text(text.text_of(start..end))
        })?
    }
}
fn crop_tile(rect: [f32; 4], size: (f32, f32)) -> Result<(f32, Tile), String> {
    let [x0, y0, x1, y1] = rect;
    if rect.iter().any(|v| !v.is_finite())
        || !size.0.is_finite()
        || !size.1.is_finite()
        || x0 < 0.0
        || y0 < 0.0
        || x1 > size.0 + 0.01
        || y1 > size.1 + 0.01
        || x1 - x0 < 2.0
        || y1 - y0 < 2.0
    {
        return Err("请在同一页内框选至少 2×2 点的有效图片区域。".into());
    }
    let (w, h) = (x1 - x0, y1 - y0);
    // 144 dpi maximum; a disclosed 2048px / 4Mpx cap bounds model payload and memory.
    let scale = 2.0_f32.min(2048.0 / w.max(h)).min((4_000_000.0 / (w * h)).sqrt());
    if !scale.is_finite() || scale < 0.01 {
        return Err("图片框选过大，请缩小范围。".into());
    }
    let x = (x0 * scale).floor() as u32;
    let y = (y0 * scale).floor() as u32;
    let right = (x1 * scale).ceil() as u32;
    let bottom = (y1 * scale).ceil() as u32;
    let tile = Tile { x, y, w: right.saturating_sub(x), h: bottom.saturating_sub(y) };
    if u64::from(tile.w) * u64::from(tile.h) > 4_100_000 {
        return Err("裁剪图超出像素限制。".into());
    }
    Ok((scale, tile))
}
impl ImageSelection {
    /// Run on a worker thread. Renders only the selected tile, without touching clipboard.
    pub fn png(self) -> Result<Vec<u8>, String> {
        crate::guard(|| {
            let (scale, tile) = crop_tile(self.rect, self.size)?;
            let mut renderer = PageRenderer::new(self.bytes, self.config);
            let out = renderer.render(RenderRequest { page: self.page, kind: RequestKind::Pixels, tile: Some(tile), scale, tag: 0, preview: false });
            if let Some(e) = out.error {
                return Err(e);
            }
            let image = image::RgbaImage::from_raw(out.width, out.height, out.rgba.to_vec()).ok_or("裁剪图片像素无效。")?;
            let image = match self.rotation {
                90 => image::imageops::rotate90(&image),
                180 => image::imageops::rotate180(&image),
                270 => image::imageops::rotate270(&image),
                _ => image,
            };
            let mut bytes = Vec::new();
            let mut encoder = png::Encoder::new(&mut bytes, image.width(), image.height());
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().and_then(|mut w| w.write_image_data(image.as_raw())).map_err(|_| "无法编码裁剪图片。")?;
            if bytes.len() > pdfcraft_ai::translation::MAX_PNG {
                return Err("裁剪图片超过 8 MB，请缩小选区。".into());
            }
            Ok(bytes)
        })?
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_bounds_are_finite_bounded_and_exact_at_integer_scale() {
        let (scale, tile) = crop_tile([10.0, 20.0, 110.0, 70.0], (200.0, 300.0)).unwrap();
        assert_eq!(scale, 2.0);
        assert_eq!((tile.x, tile.y, tile.w, tile.h), (20, 40, 200, 100));
        for rect in [[f32::NAN, 0.0, 20.0, 20.0], [-1.0, 0.0, 10.0, 10.0], [0.0, 0.0, 0.0, 2.0], [0.0, 0.0, 201.0, 20.0]] {
            assert!(crop_tile(rect, (200.0, 300.0)).is_err());
        }
    }
}
