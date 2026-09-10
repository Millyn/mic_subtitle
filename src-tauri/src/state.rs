use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU16};
use std::sync::{mpsc::Sender, Arc, Mutex};
use tokio::sync::{broadcast, oneshot, RwLock};
use uuid::Uuid;

pub const DEFAULT_SERVER_PORT: u16 = 39071;
pub const DEFAULT_CANVAS_WIDTH: u32 = 1920;
pub const DEFAULT_CANVAS_HEIGHT: u32 = 1080;
pub const DEFAULT_SUBTITLE_X: u32 = DEFAULT_CANVAS_WIDTH / 2;
pub const DEFAULT_SUBTITLE_Y: u32 = 900;

pub fn default_server_port() -> u16 {
    DEFAULT_SERVER_PORT
}

pub fn normalize_server_port(port: u16) -> u16 {
    if port < 1024 {
        DEFAULT_SERVER_PORT
    } else {
        port
    }
}

fn default_canvas_width() -> u32 {
    DEFAULT_CANVAS_WIDTH
}

fn default_canvas_height() -> u32 {
    DEFAULT_CANVAS_HEIGHT
}

fn default_subtitle_x() -> u32 {
    DEFAULT_SUBTITLE_X
}

fn default_subtitle_y() -> u32 {
    DEFAULT_SUBTITLE_Y
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextStyle {
    pub font_family: String,
    pub font_size: u32,
    pub color: String,
    pub opacity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleStyle {
    pub chinese: TextStyle,
    pub english: TextStyle,
    pub layout: String,
    pub alignment: String,
    pub position: String,
    #[serde(default = "default_canvas_width")]
    pub canvas_width: u32,
    #[serde(default = "default_canvas_height")]
    pub canvas_height: u32,
    #[serde(default = "default_subtitle_x")]
    pub subtitle_x: u32,
    #[serde(default = "default_subtitle_y")]
    pub subtitle_y: u32,
    pub max_width: u32,
    pub line_spacing: f32,
    pub show_temporary: bool,
    pub show_final: bool,
    pub background_color: String,
    pub background_opacity: f32,
    pub outline_width: u32,
    pub outline_color: String,
    pub shadow: bool,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            chinese: TextStyle {
                font_family: "Microsoft YaHei, PingFang SC, sans-serif".into(),
                font_size: 34,
                color: "#ffffff".into(),
                opacity: 1.0,
            },
            english: TextStyle {
                font_family: "Segoe UI, Arial, sans-serif".into(),
                font_size: 23,
                color: "#a7f3d0".into(),
                opacity: 0.94,
            },
            layout: "stacked".into(),
            alignment: "center".into(),
            position: "bottom".into(),
            canvas_width: DEFAULT_CANVAS_WIDTH,
            canvas_height: DEFAULT_CANVAS_HEIGHT,
            subtitle_x: DEFAULT_SUBTITLE_X,
            subtitle_y: DEFAULT_SUBTITLE_Y,
            max_width: 1100,
            line_spacing: 1.3,
            show_temporary: true,
            show_final: true,
            background_color: "#07111f".into(),
            background_opacity: 0.68,
            outline_width: 2,
            outline_color: "#020617".into(),
            shadow: true,
        }
    }
}

impl SubtitleStyle {
    pub fn normalize(&mut self) {
        self.canvas_width = self.canvas_width.clamp(320, 16_384);
        self.canvas_height = self.canvas_height.clamp(180, 8_640);
        self.subtitle_x = self.subtitle_x.min(self.canvas_width);
        self.subtitle_y = self.subtitle_y.min(self.canvas_height);
        self.max_width = self.max_width.clamp(200, self.canvas_width);
        self.line_spacing = self.line_spacing.clamp(0.8, 3.0);
        self.background_opacity = self.background_opacity.clamp(0.0, 1.0);
        self.chinese.opacity = self.chinese.opacity.clamp(0.0, 1.0);
        self.english.opacity = self.english.opacity.clamp(0.0, 1.0);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepSeekConfig {
    pub enabled: bool,
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

impl Default for DeepSeekConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            api_key: String::new(),
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-chat".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModel {
    pub id: String,
    #[serde(default)]
    pub version: Option<String>,
    pub installed: bool,
    pub total_size: u64,
    pub sha256: Option<String>,
    pub verified_at: Option<String>,
    pub files: Vec<ModelFileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFileRecord {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub selected_device: Option<String>,
    pub active_model: String,
    pub deepseek: DeepSeekConfig,
    pub style: SubtitleStyle,
    #[serde(default = "default_server_port")]
    pub server_port: u16,
    #[serde(default)]
    pub models: HashMap<String, InstalledModel>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            selected_device: None,
            active_model: "qwen3-asr-0.6b".into(),
            deepseek: DeepSeekConfig::default(),
            style: SubtitleStyle::default(),
            server_port: DEFAULT_SERVER_PORT,
            models: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleEvent {
    pub id: String,
    pub timestamp: u64,
    pub kind: String,
    pub chinese: String,
    #[serde(default)]
    pub english: Option<String>,
    #[serde(default)]
    pub translation_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenCounter {
    pub asr_utterances: u64,
    pub asr_tokens: u64,
    pub asr_estimated: bool,
    pub translation_requests: u64,
    pub translation_prompt_tokens: u64,
    pub translation_completion_tokens: u64,
    pub translation_total_tokens: u64,
    pub translation_estimated: bool,
    pub total_tokens: u64,
}

impl TokenCounter {
    fn add_asr(&mut self, tokens: u64, estimated: bool) {
        self.asr_utterances = self.asr_utterances.saturating_add(1);
        self.asr_tokens = self.asr_tokens.saturating_add(tokens);
        self.total_tokens = self.total_tokens.saturating_add(tokens);
        self.asr_estimated |= estimated;
    }

    fn add_translation(
        &mut self,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        estimated: bool,
    ) {
        let total_tokens = if total_tokens == 0 {
            prompt_tokens.saturating_add(completion_tokens)
        } else {
            total_tokens
        };
        self.translation_requests = self.translation_requests.saturating_add(1);
        self.translation_prompt_tokens = self
            .translation_prompt_tokens
            .saturating_add(prompt_tokens);
        self.translation_completion_tokens = self
            .translation_completion_tokens
            .saturating_add(completion_tokens);
        self.translation_total_tokens = self
            .translation_total_tokens
            .saturating_add(total_tokens);
        self.translation_estimated |= estimated;
        self.total_tokens = self.total_tokens.saturating_add(total_tokens);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub session: TokenCounter,
    pub all_time: TokenCounter,
}

impl SubtitleEvent {
    pub fn new(kind: impl Into<String>, chinese: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: now_millis(),
            kind: kind.into(),
            chinese: chinese.into(),
            english: None,
            translation_error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WsMessage {
    Init {
        style: SubtitleStyle,
        subtitle: Option<SubtitleEvent>,
    },
    Style {
        style: SubtitleStyle,
    },
    Subtitle {
        event: SubtitleEvent,
    },
    Status {
        status: String,
    },
}

#[derive(Clone)]
pub struct AppState {
    pub config_path: Arc<PathBuf>,
    pub models_path: Arc<PathBuf>,
    pub config: Arc<RwLock<AppConfig>>,
    pub subtitle_tx: broadcast::Sender<WsMessage>,
    pub latest_subtitle: Arc<RwLock<Option<SubtitleEvent>>>,
    pub config_save_lock: Arc<tokio::sync::Mutex<()>>,
    pub server_port: Arc<AtomicU16>,
    pub server_started: Arc<AtomicBool>,
    pub server_stop: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    pub audio_stop: Arc<Mutex<Option<Sender<()>>>>,
    pub recognition_capture: Arc<Mutex<Option<crate::audio::RecognitionCapture>>>,
    pub recognition_child: Arc<Mutex<Option<Child>>>,
    pub download_cancellations: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    pub token_usage: Arc<RwLock<TokenUsage>>,
}

impl AppState {
    pub fn new(config_path: PathBuf, models_path: PathBuf) -> Result<Self, String> {
        let (subtitle_tx, _) = broadcast::channel(64);
        let config = read_config(&config_path)?;
        let server_port = normalize_server_port(config.server_port);
        Ok(Self {
            config_path: Arc::new(config_path),
            models_path: Arc::new(models_path),
            config: Arc::new(RwLock::new(config)),
            subtitle_tx,
            latest_subtitle: Arc::new(RwLock::new(None)),
            config_save_lock: Arc::new(tokio::sync::Mutex::new(())),
            server_port: Arc::new(AtomicU16::new(server_port)),
            server_started: Arc::new(AtomicBool::new(false)),
            server_stop: Arc::new(Mutex::new(None)),
            audio_stop: Arc::new(Mutex::new(None)),
            recognition_capture: Arc::new(Mutex::new(None)),
            recognition_child: Arc::new(Mutex::new(None)),
            download_cancellations: Arc::new(Mutex::new(HashMap::new())),
            token_usage: Arc::new(RwLock::new(TokenUsage::default())),
        })
    }

    pub fn models_dir(&self) -> PathBuf {
        self.models_path.as_ref().clone()
    }

    pub fn model_dir(&self, model_id: &str) -> PathBuf {
        self.models_dir().join(model_id)
    }

    pub async fn save_config(&self) -> Result<(), String> {
        let _save_guard = self.config_save_lock.lock().await;
        let config = self.config.read().await.clone();
        let path = self.config_path.as_ref();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| error.to_string())?;
        }
        let data = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
        let temp_path = path.with_extension("json.tmp");
        tokio::fs::write(&temp_path, data)
            .await
            .map_err(|error| error.to_string())?;
        if tokio::fs::try_exists(path)
            .await
            .map_err(|error| error.to_string())?
        {
            // Windows does not allow rename-overwrite; the save lock keeps this
            // replacement serialized with other config writes.
            tokio::fs::remove_file(path)
                .await
                .map_err(|error| error.to_string())?;
        }
        tokio::fs::rename(&temp_path, path)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn publish_subtitle(&self, event: SubtitleEvent) {
        *self.latest_subtitle.write().await = Some(event.clone());
        let _ = self.subtitle_tx.send(WsMessage::Subtitle { event });
    }

    pub async fn publish_style(&self, style: SubtitleStyle) {
        let _ = self.subtitle_tx.send(WsMessage::Style { style });
    }

    pub async fn init_message(&self) -> WsMessage {
        let style = self.config.read().await.style.clone();
        let subtitle = self.latest_subtitle.read().await.clone();
        WsMessage::Init { style, subtitle }
    }

    pub async fn get_token_usage(&self) -> TokenUsage {
        self.token_usage.read().await.clone()
    }

    pub async fn reset_token_session(&self) -> TokenUsage {
        let mut usage = self.token_usage.write().await;
        usage.session = TokenCounter::default();
        usage.clone()
    }

    pub async fn record_asr_tokens(&self, tokens: u64, estimated: bool) -> TokenUsage {
        let mut usage = self.token_usage.write().await;
        usage.session.add_asr(tokens, estimated);
        usage.all_time.add_asr(tokens, estimated);
        usage.clone()
    }

    pub async fn record_translation_tokens(
        &self,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        estimated: bool,
    ) -> TokenUsage {
        let mut usage = self.token_usage.write().await;
        usage
            .session
            .add_translation(prompt_tokens, completion_tokens, total_tokens, estimated);
        usage
            .all_time
            .add_translation(prompt_tokens, completion_tokens, total_tokens, estimated);
        usage.clone()
    }
}

fn read_config(path: &Path) -> Result<AppConfig, String> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let mut config: AppConfig =
        serde_json::from_slice(&bytes).map_err(|error| format!("配置文件损坏：{error}"))?;
    config.style.normalize();
    Ok(config)
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

pub fn sha256_file(path: &Path) -> Result<(u64, String), String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    let mut size = 0_u64;
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((size, hex::encode(hasher.finalize())))
}
