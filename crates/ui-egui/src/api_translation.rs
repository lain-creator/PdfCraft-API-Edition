//! The custom edition's opt-in translation bar and read-only result window.
use crate::{PdfCraftApp, QuickTool};
use pdfcraft_ai::translation::{Direction, Source, Translation};
use pdfcraft_ai::{Authentication, Protocol, Settings};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
#[derive(Default)]
pub(crate) struct State {
    pub settings_open: bool,
    open: bool,
    loaded: bool,
    settings: Settings,
    source: Option<Source>,
    source_doc: Option<pdfcraft_engine::DocId>,
    origin: String,
    result: Option<Translation>,
    preview: Option<egui::TextureHandle>,
    error: String,
    glossary: String,
    status: String,
    job: Option<Job>,
}
struct Job {
    doc: pdfcraft_engine::DocId,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Outcome, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
enum Outcome {
    Image(Vec<u8>),
    Translation(Translation),
}
pub(crate) fn toolbar(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    // Use the existing 48pt chrome: an extra row shrinks pages/thumbnails and can
    // put otherwise visible drawing targets below the viewport. Narrow windows
    // retain the same actions in a menu without overflowing the mode bar.
    if ui.available_width() < 800.0 {
        ui.menu_button("API 翻译", |ui| controls(app, ui));
    } else {
        controls(app, ui);
    }
}
fn controls(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    if ui.button("API 设置").clicked() {
        app.run_command("api.settings");
    }
    ui.separator();
    if ui.selectable_label(app.quick_tool == QuickTool::Select, "文字选区").clicked() {
        app.run_command("api.text");
    }
    if ui.selectable_label(app.quick_tool == QuickTool::TranslateImage, "图片框选").clicked() {
        app.run_command("api.image");
    }
    for (id, label) in [("api.zh_en", "中译英"), ("api.en_zh", "英译中")] {
        if ui
            .add_enabled(app.active.is_some() && app.api_translation.job.is_none(), egui::Button::new(label))
            .on_hover_text("仅发送选中文字或裁剪图；可能产生 API 费用，不修改 PDF。")
            .clicked()
        {
            app.run_command(id);
        }
    }
    if app.api_translation.source.is_some() && ui.button("查看译文").clicked() {
        app.api_translation.open = true;
    }
}
impl PdfCraftApp {
    pub(crate) fn open_api_settings(&mut self) {
        self.load_api_settings();
        self.api_translation.settings_open = true;
    }
    fn load_api_settings(&mut self) {
        if self.api_translation.loaded {
            return;
        }
        self.api_translation.loaded = true;
        #[cfg(not(target_arch = "wasm32"))]
        {
            match pdfcraft_ai::transport::load_settings() {
                Ok(settings) => self.api_translation.settings = settings,
                Err(e) => self.api_translation.error = e,
            }
            self.api_translation.glossary = match pdfcraft_ai::pcb_glossary::manage(None) {
                Ok(info) => info.label(),
                Err(e) => e,
            };
        }
    }
    pub(crate) fn translate_api_selection(&mut self, direction: Direction) {
        if self.api_translation.job.is_some() {
            return;
        }
        let Some(index) = self.active else {
            return;
        };
        let Some(view) = self.views.get(index) else {
            return;
        };
        let id = view.id;
        if self.quick_tool == QuickTool::TranslateImage {
            if self.api_translation.source_doc != Some(id) || !matches!(self.api_translation.source, Some(Source::Image(_))) {
                self.notify("请先拖动框选图片区域，再点击翻译。");
                return;
            }
        } else {
            // Select All spans unread pages. Refuse explicitly instead of translating a
            // cached subset and claiming it was the full selection.
            if view.all_text {
                self.notify("请拖动选择当前页的文字后翻译；暂不支持跨页全选，未发送任何内容。");
                return;
            }
            let Some(text) = view.selected_text() else {
                self.notify("请先拖动选择 PDF 文字，或切换到图片框选。");
                return;
            };
            let Some(doc) = self.session.get(id) else {
                return;
            };
            match doc.translation_text(text) {
                Ok(source) => {
                    self.api_translation.source = Some(source);
                    self.api_translation.source_doc = Some(id);
                    self.api_translation.origin = doc.name.clone();
                    self.api_translation.result = None;
                    self.api_translation.preview = None;
                }
                Err(e) => {
                    self.notify(e);
                    return;
                }
            }
        }
        self.start_api_translation(direction);
    }
    pub(crate) fn prepare_api_image(&mut self, index: usize, page: usize, rect: [f32; 4]) {
        let Some(view) = self.views.get(index) else {
            return;
        };
        let id = view.id;
        let selection = match self.session.translation_image(id, page, rect, view.rotation) {
            Ok(source) => source,
            Err(e) => {
                self.notify(e);
                return;
            }
        };
        let origin = self.session.get(id).map(|d| d.name.clone()).unwrap_or_default();
        self.api_translation.job = None;
        self.api_translation.source = None;
        self.api_translation.result = None;
        self.api_translation.preview = None;
        self.api_translation.source_doc = Some(id);
        self.api_translation.origin = origin;
        self.api_translation.open = true;
        self.api_translation.error.clear();
        self.api_translation.status = "正在本机生成裁剪预览，尚未发送 API…".into();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = mpsc::channel();
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let result = selection.png().map(Outcome::Image);
                if !flag.load(Ordering::Relaxed) {
                    let _ = tx.send(result);
                }
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
            self.api_translation.job = Some(Job { doc: id, cancel, rx });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = selection;
            self.api_translation.error = "API 翻译目前仅支持桌面版。".into();
        }
    }
    fn start_api_translation(&mut self, direction: Direction) {
        self.load_api_settings();
        self.api_translation.open = true;
        self.api_translation.error.clear();
        let Some(id) = self.api_translation.source_doc else {
            return;
        };
        let Some(source) = self.api_translation.source.clone() else {
            return;
        };
        // Recheck permissions before sending, including after an edit/security change.
        if self.session.get(id).is_none_or(|d| !d.allows_copying()) {
            self.api_translation.error = "文档已关闭或不允许复制内容，未发送。".into();
            return;
        }
        let profile = match self.api_translation.settings.selected() {
            Ok(p) => p.clone(),
            Err(e) => {
                self.api_translation.error = e;
                return;
            }
        };
        self.api_translation.result = None;
        self.api_translation.status = if matches!(source, Source::Image(_)) {
            "正在识别框选图片，然后按专业术语翻译（两次 API）…"
        } else {
            "正在翻译选中文字（一次 API）…"
        }
        .into();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = mpsc::channel();
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let result = pdfcraft_ai::translation::translate(source, direction, profile, &flag).map(Outcome::Translation);
                if !flag.load(Ordering::Relaxed) {
                    let _ = tx.send(result);
                }
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
            self.api_translation.job = Some(Job { doc: id, cancel, rx });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (profile, source, direction);
            self.api_translation.error = "API 翻译目前仅支持桌面版。".into();
        }
    }
    pub(crate) fn poll_api_translation(&mut self, ctx: &egui::Context) {
        let current = self.active.and_then(|i| self.views.get(i)).map(|v| v.id);
        if self.api_translation.source_doc.is_some() && self.api_translation.source_doc != current {
            self.api_translation =
                State { settings: std::mem::take(&mut self.api_translation.settings), loaded: self.api_translation.loaded, ..Default::default() };
            return;
        }
        let Some(job) = &self.api_translation.job else {
            return;
        };
        if current != Some(job.doc) {
            self.api_translation.job = None;
            return;
        }
        match job.rx.try_recv() {
            Ok(outcome) => {
                self.api_translation.job = None;
                self.api_translation.status.clear();
                match outcome {
                    Ok(Outcome::Image(png)) => {
                        if let Ok(image) = decode_preview(&png) {
                            self.api_translation.preview = Some(ctx.load_texture("api-crop", image, egui::TextureOptions::LINEAR));
                        }
                        self.api_translation.source = Some(Source::Image(png));
                    }
                    Ok(Outcome::Translation(result)) => self.api_translation.result = Some(result),
                    Err(e) => self.api_translation.error = e,
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.api_translation.job = None;
                self.api_translation.status.clear();
                self.api_translation.error = "后台任务意外结束；未改动 PDF，未自动重试。".into();
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
        }
    }
    pub(crate) fn api_translation_windows(&mut self, ctx: &egui::Context) {
        if self.api_translation.settings_open {
            self.api_settings_window(ctx);
        }
        if !self.api_translation.open {
            return;
        }
        let mut open = true;
        let mut direction = None;
        let state = &mut self.api_translation;
        egui::Window::new("PDF API 翻译")
            .id(egui::Id::new("pdf_api_result"))
            .open(&mut open)
            .default_size(egui::vec2(720.0, 600.0))
            .resizable(true)
            .collapsible(true)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.label(format!("来源: {}", state.origin));
                ui.small("结果仅显示于此窗口，不写回 PDF。关闭窗口会停止本机等待，服务商可能已处理已发送的请求。");
                let image = matches!(state.source, Some(Source::Image(_)));
                if image {
                    ui.label("图片模式: 先视觉识别，再按匹配术语翻译，共两次 API；仅发送裁剪图，不上传整份 PDF。");
                    ui.small("裁剪图最高 144 dpi，长边 2048 像素；小字不清晰时请缩小框选区域。");
                    if let Some(preview) = &state.preview {
                        ui.add(egui::Image::new(preview).max_size(egui::vec2(660.0, 220.0)));
                    }
                }
                ui.horizontal(|ui| {
                    for d in [Direction::ZhEn, Direction::EnZh] {
                        if ui.add_enabled(state.source.is_some() && state.job.is_none(), egui::Button::new(d.label())).clicked() {
                            direction = Some(d);
                        }
                    }
                    if state.job.is_some() && ui.button("取消").clicked() {
                        state.job = None;
                        state.status = "已停止本机等待；已发送请求可能仍在服务端运行。".into();
                    }
                });
                if !state.status.is_empty() {
                    ui.label(&state.status);
                }
                if !state.error.is_empty() {
                    ui.label(&state.error);
                }
                if let Some(result) = &state.result {
                    ui.separator();
                    ui.collapsing("识别原文 / 选区原文", |ui| {
                        ui.add(egui::Label::new(&result.source_text).selectable(true).wrap());
                        if ui.button("复制原文").clicked() {
                            ctx.copy_text(result.source_text.clone());
                        }
                    });
                    ui.heading("译文");
                    ui.add(egui::Label::new(&result.translation).selectable(true).wrap());
                    if ui.button("复制译文").clicked() {
                        ctx.copy_text(result.translation.clone());
                    }
                    ui.small(format!("匹配专业术语: {} 条", result.matched_terms));
                    if !result.notes.is_empty() {
                        ui.separator();
                        ui.label(format!("核对提示: {}", result.notes));
                    }
                } else if let Some(Source::Text(text)) = &state.source {
                    ui.separator();
                    ui.add(egui::Label::new(text).selectable(true).wrap());
                }
            });
        if !open {
            self.api_translation.open = false;
            self.api_translation.job = None;
        }
        if let Some(d) = direction {
            self.start_api_translation(d);
        }
    }
    fn api_settings_window(&mut self, ctx: &egui::Context) {
        self.load_api_settings();
        let mut open = true;
        let state = &mut self.api_translation;
        let mut save = false;
        let mut refresh = false;
        egui::Window::new("PDF API 设置")
            .id(egui::Id::new("pdf_api_settings"))
            .open(&mut open)
            .default_width(620.0)
            .default_height(680.0)
            .resizable(true)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.label("仅点击翻译才将选区发送至下方服务商。文字一次 API，图片识别加翻译两次 API，可能产生费用。");
                let selected = state.settings.selected().map(|p| p.name.as_str()).unwrap_or("请选择");
                egui::ComboBox::from_id_salt("api_provider").selected_text(selected).show_ui(ui, |ui| {
                    for (i, p) in state.settings.profiles.iter().enumerate() {
                        ui.selectable_value(&mut state.settings.active, i, &p.name);
                    }
                });
                if let Some(p) = state.settings.profiles.get_mut(state.settings.active) {
                    ui.label("API 基础地址");
                    ui.text_edit_singleline(&mut p.base_url);
                    ui.label("文字模型 ID");
                    ui.text_edit_singleline(&mut p.model);
                    ui.label("视觉模型 ID (必须支持图片输入；不确定时请向服务商确认)");
                    ui.text_edit_singleline(&mut p.vision_model);
                    ui.label("API Key");
                    ui.add(egui::TextEdit::singleline(&mut p.api_key).password(true));
                    ui.add(crate::widgets::checkbox(&mut p.remember_key, "记住 API Key (Mac 钥匙串 / Windows 凭据管理器)"));
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut p.protocol, Protocol::ChatCompletions, "Chat Completions");
                        ui.selectable_value(&mut p.protocol, Protocol::Responses, "Responses");
                    });
                    ui.collapsing("高级设置", |ui| {
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut p.authentication, Authentication::Bearer, "Bearer");
                            ui.selectable_value(&mut p.authentication, Authentication::ApiKey, "api-key");
                        });
                        ui.add(crate::widgets::checkbox(&mut p.allow_http, "明确允许远程 HTTP (内容及密钥未经加密)"));
                        ui.add(egui::Slider::new(&mut p.timeout_seconds, 10..=600).text("超时 (秒)"));
                        ui.label("附加请求参数 JSON (不能覆盖消息/模型/工具)");
                        ui.add(egui::TextEdit::multiline(&mut p.extra_body).code_editor().desired_rows(3));
                    });
                }
                ui.separator();
                ui.label("PCB 专业术语库 (内置 + 本机补充)");
                ui.label(&state.glossary);
                ui.small("公开包内置 798 条 PCB 术语；本机词库独立保留。只发送命中词条，不依赖原网站。");
                if ui.button("刷新词库状态").clicked() {
                    refresh = true;
                }
                ui.separator();
                if ui.button("保存设置").clicked() {
                    save = true;
                }
                ui.label(&state.error);
            });
        self.api_translation.settings_open = open;
        #[cfg(not(target_arch = "wasm32"))]
        {
            if save {
                self.api_translation.error = match pdfcraft_ai::transport::save_settings(&self.api_translation.settings) {
                    Ok(()) => "已保存；API Key 不写入普通 JSON 配置。".into(),
                    Err(e) => e,
                };
            }
            if refresh {
                self.api_translation.glossary = match pdfcraft_ai::pcb_glossary::manage(None) {
                    Ok(i) => i.label(),
                    Err(e) => e,
                };
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (save, refresh);
        }
    }
}
fn decode_preview(png: &[u8]) -> Result<egui::ColorImage, String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().map_err(|_| "预览图片无效。")?;
    let size = reader.output_buffer_size().filter(|n| *n <= 4_100_000 * 4).ok_or("预览图片过大。")?;
    let mut bytes = vec![0; size];
    let info = reader.next_frame(&mut bytes).map_err(|_| "无法解码预览。")?;
    let pixels = bytes.get(..info.buffer_size()).ok_or("预览像素不足。")?;
    if info.color_type != png::ColorType::Rgba {
        return Err("预览像素格式无效。".into());
    }
    Ok(egui::ColorImage::from_rgba_unmultiplied([info.width as usize, info.height as usize], pixels))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn text_and_image_results_render_in_chinese_without_editing_the_pdf() {
        for image_mode in [false, true] {
            let mut h = Harness::builder().with_size(egui::vec2(1440.0, 1000.0)).build_eframe(move |cc| {
                let mut app = PdfCraftApp::new();
                app.set_option("language", "zh-hans").unwrap();
                let source = "Lamination structure: core thickness 1.6mm.";
                let bytes = app.session.create_from_text("Synthetic API translation QA", source).unwrap();
                app.open_bytes("synthetic-qa.pdf", None, bytes.as_ref().clone()).unwrap();
                let id = app.views[0].id;
                let state = &mut app.api_translation;
                state.open = true;
                state.loaded = true;
                state.source_doc = Some(id);
                state.origin = "synthetic-qa.pdf (合成测试文档)".into();
                state.source = Some(if image_mode {
                    let png = app.session.translation_image(id, 0, [20.0, 20.0, 580.0, 200.0], 0).unwrap().png().unwrap();
                    state.preview = Some(cc.egui_ctx.load_texture("test-crop", decode_preview(&png).unwrap(), egui::TextureOptions::LINEAR));
                    Source::Image(png)
                } else {
                    Source::Text(source.into())
                });
                let (tx, rx) = mpsc::channel();
                tx.send(Ok(Outcome::Translation(Translation {
                    source_text: source.into(),
                    translation: "压合结构: 芯板厚度 1.6mm。".into(),
                    notes: "这是合成界面测试结果，未调用真实 API。".into(),
                    matched_terms: 2,
                })))
                .unwrap();
                state.job = Some(Job { doc: id, cancel: Arc::new(AtomicBool::new(false)), rx });
                app
            });
            h.run_steps(8);
            h.get_by_label("复制译文");
            h.get_by_label("压合结构: 芯板厚度 1.6mm。");
            assert!(h.state().api_translation.result.is_some());
            assert!(h.state().api_translation.job.is_none());
            let doc = h.state().session.get(h.state().views[0].id).unwrap();
            assert!(!doc.dirty);
            assert!(doc.can_undo().is_none());
            if let Ok(dir) = std::env::var("PDFCRAFT_API_QA_DIR") {
                let name = if image_mode { "image-result" } else { "text-result" };
                h.render().unwrap().save(std::path::Path::new(&dir).join(format!("{name}.png"))).unwrap();
            }
        }
    }
    #[test]
    fn switching_document_cancels_waiting_and_keeps_profile_without_sending() {
        let mut app = PdfCraftApp::new();
        app.api_translation.loaded = true;
        app.api_translation.settings.active = 2;
        app.api_translation.source_doc = Some(pdfcraft_engine::DocId(99));
        app.api_translation.source = Some(Source::Text("synthetic frozen source".into()));
        let (_, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        app.api_translation.job = Some(Job { doc: pdfcraft_engine::DocId(99), cancel: cancel.clone(), rx });
        app.poll_api_translation(&egui::Context::default());
        assert!(cancel.load(Ordering::Relaxed));
        assert!(app.api_translation.job.is_none() && app.api_translation.source.is_none());
        assert_eq!(app.api_translation.settings.active, 2);
        assert!(app.api_translation.loaded);
    }
    #[test]
    fn preview_decodes_only_bounded_rgba_and_closing_job_cancels() {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 16, 8);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&[255; 16 * 8 * 4]).unwrap();
        assert_eq!(decode_preview(&bytes).unwrap().size, [16, 8]);
        assert!(decode_preview(b"not an image").is_err());
        let (_, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        drop(Job { doc: pdfcraft_engine::DocId(1), cancel: cancel.clone(), rx });
        assert!(cancel.load(Ordering::Relaxed));
    }
}
