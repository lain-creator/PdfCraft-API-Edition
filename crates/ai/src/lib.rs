//! Opt-in PDF translation; no document mutations and no automatic network requests.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! User-configured API profiles; secrets are excluded from JSON and document data.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    #[default]
    ChatCompletions,
    Responses,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Authentication {
    #[default]
    Bearer,
    ApiKey,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub vision_model: String,
    pub protocol: Protocol,
    pub authentication: Authentication,
    pub extra_body: String,
    pub timeout_seconds: u32,
    pub allow_http: bool,
    pub remember_key: bool,
    /// Never part of JSON preferences, workbook data, logs or control responses.
    #[serde(skip)]
    pub api_key: String,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            id: "custom".into(),
            name: "自定义兼容 API".into(),
            base_url: String::new(),
            model: String::new(),
            vision_model: String::new(),
            protocol: Protocol::ChatCompletions,
            authentication: Authentication::Bearer,
            extra_body: "{}".into(),
            timeout_seconds: 180,
            allow_http: false,
            remember_key: false,
            api_key: String::new(),
        }
    }
}
impl Profile {
    pub fn presets() -> Vec<Self> {
        [
            ("openai", "GPT / OpenAI", "https://api.openai.com/v1", "gpt-4.1-mini"),
            ("minimax", "MiniMax", "https://api.minimax.io/v1", "MiniMax-M2.7"),
            ("deepseek", "DeepSeek", "https://api.deepseek.com/v1", "deepseek-chat"),
            ("kimi", "Kimi / 月之暗面", "https://api.moonshot.cn/v1", "kimi-k2.5"),
            ("xiaomi", "小米 MiMo", "https://api.xiaomimimo.com/v1", "mimo-v2.6-flash"),
            ("custom", "自定义兼容 API", "", ""),
        ]
        .into_iter()
        .map(|(id, name, base_url, model)| Self {
            id: id.into(),
            name: name.into(),
            base_url: base_url.into(),
            model: model.into(),
            vision_model: if id == "openai" { model.into() } else { String::new() },
            protocol: if id == "openai" { Protocol::Responses } else { Protocol::ChatCompletions },
            authentication: if id == "xiaomi" { Authentication::ApiKey } else { Authentication::Bearer },
            extra_body: if id == "minimax" { "{\"reasoning_split\":true}".into() } else { "{}".into() },
            ..Self::default()
        })
        .collect()
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub active: usize,
    pub profiles: Vec<Profile>,
}
impl Default for Settings {
    fn default() -> Self {
        Self { active: 0, profiles: Profile::presets() }
    }
}
impl Settings {
    pub fn selected(&self) -> Result<&Profile, String> {
        self.profiles.get(self.active).ok_or_else(|| "请选择 API 服务商.".into())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: serde_json::Value,
}
#[derive(Clone, Default, Debug, Serialize)]
pub struct GlossaryInfo {
    pub entries: usize,
    pub quarantined: usize,
    pub ambiguous: usize,
    pub source: String,
}
impl GlossaryInfo {
    pub fn label(&self) -> String {
        if self.entries == 0 {
            "未导入本机 PCB 词库.".into()
        } else {
            format!(
                "已导入 {} 条; {} 条待核对不参与匹配, {} 组多义词按上下文判断. 来源: {}",
                self.entries, self.quarantined, self.ambiguous, self.source
            )
        }
    }
}

pub mod pcb_glossary;
pub mod translation;
#[cfg(not(target_arch = "wasm32"))]
#[path = "ai_api.rs"]
pub mod transport;
/// The custom edition stores only its own API settings and glossary here.
pub fn config_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    return std::env::var_os("HOME").map(std::path::PathBuf::from).map(|p| p.join("Library/Application Support/PdfCraftAPI"));
    #[cfg(target_os = "windows")]
    return std::env::var_os("APPDATA").map(std::path::PathBuf::from).map(|p| p.join("PdfCraftAPI"));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from).map(|p| p.join(".config")))
        .map(|p| p.join("pdfcraft-api"))
}
pub fn prefs_enabled() -> bool {
    std::env::var_os("PDFCRAFT_NO_PREFS").is_none() && !cfg!(target_arch = "wasm32")
}
