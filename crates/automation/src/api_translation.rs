//! Explicit consent is mandatory for every headless outbound request. No implicit
//! reads of the desktop's personal settings, keys or glossary outside --root.
use crate::{Args, Automation, Content, Result, ToolError, failed};
use pdfcraft_ai::translation::{Direction, Source};
use serde_json::{Value, json};
impl Automation {
    fn api_source(&self, a: &Args) -> Result<Source> {
        let id = self.doc_to_copy(a)?.id;
        let page = self.page(a)?;
        match a.opt_str("mode")?.unwrap_or("text") {
            "text" => {
                let start = usize::try_from(a.int("start")?).map_err(|_| failed("start must be a zero-based glyph index"))?;
                let end = usize::try_from(a.int("end")?).map_err(|_| failed("end must be an exclusive glyph index"))?;
                self.session.translation_glyphs(id, page, start, end).map_err(failed)
            }
            "image" => {
                let rect = a.nums::<4>("rect")?.ok_or_else(|| failed("rect is required for image mode"))?.map(|v| v as f32);
                let rotation = u16::try_from(a.opt_int("rotation")?.unwrap_or(0)).map_err(|_| failed("invalid rotation"))?;
                self.session.translation_image(id, page, rect, rotation).and_then(|s| s.png()).map(Source::Image).map_err(failed)
            }
            _ => Err(ToolError::InvalidArgs("mode must be text or image".into())),
        }
    }
    pub(super) fn api_selection_text(&self, a: &Args) -> Result<Value> {
        if a.opt_str("mode")?.is_some_and(|v| v != "text") {
            return Err(failed("mode must be text"));
        }
        match self.api_source(a)? {
            Source::Text(text) => Ok(json!({"text":text,"sent":false})),
            _ => Err(failed("expected text selection")),
        }
    }
    pub(super) fn api_selection_image(&self, a: &Args) -> Result<Vec<Content>> {
        if a.opt_str("mode")? != Some("image") {
            return Err(failed("mode must be image"));
        }
        match self.api_source(a)? {
            Source::Image(data) => {
                let decoder = png::Decoder::new(std::io::Cursor::new(&data));
                let reader = decoder.read_info().map_err(failed)?;
                let info = reader.info();
                Ok(vec![
                    Content::Json(json!({"sent":false,"width":info.width,"height":info.height})),
                    Content::Png { width: info.width, height: info.height, data },
                ])
            }
            _ => Err(failed("expected image selection")),
        }
    }
    pub(super) fn api_translate(&self, a: &Args) -> Result<Value> {
        if a.opt_bool("allow_api_send")? != Some(true) {
            return Err(failed("allow_api_send: true is required; selected text/crop will be sent to the configured provider and may incur fees"));
        }
        let source = self.api_source(a)?;
        let direction = match a.str("direction")? {
            "zh_en" => Direction::ZhEn,
            "en_zh" => Direction::EnZh,
            _ => return Err(failed("direction must be zh_en or en_zh")),
        };
        let raw = a.get("profile").filter(|p| p.is_object()).ok_or_else(|| failed("profile must be an API profile object"))?;
        let mut profile: pdfcraft_ai::Profile = serde_json::from_value(raw.clone()).map_err(|_| failed("invalid API profile"))?;
        if let Some(name) = a.opt_str("api_key_env")? {
            if name.is_empty() || name.len() > 128 || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
                return Err(failed("api_key_env must name an environment variable"));
            }
            profile.api_key = std::env::var(name).map_err(|_| failed("API key environment variable is unavailable"))?;
        }
        let glossary = if let Some(path) = a.opt_str("glossary_path")? {
            let path = self.resolve(path, false)?;
            let file = std::fs::File::open(path).map_err(failed)?;
            use std::io::Read;
            let mut bytes = Vec::new();
            file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(failed)?;
            pdfcraft_ai::pcb_glossary::Glossary::with_local(pdfcraft_ai::pcb_glossary::Glossary::parse(&bytes).map_err(failed)?).map_err(failed)?
        } else {
            pdfcraft_ai::pcb_glossary::Glossary::builtin().map_err(failed)?
        };
        #[cfg(not(target_arch = "wasm32"))]
        let result = pdfcraft_ai::translation::translate_with(
            source,
            direction,
            profile,
            &glossary,
            &std::sync::atomic::AtomicBool::new(false),
            &pdfcraft_ai::transport::complete,
        )
        .map_err(failed)?;
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (source, direction, profile, glossary);
            return Err(failed("API translation is desktop-only"));
        }
        #[cfg(not(target_arch = "wasm32"))]
        serde_json::to_value(result).map_err(failed)
    }
}
