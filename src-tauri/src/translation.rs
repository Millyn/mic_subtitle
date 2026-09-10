use std::future::Future;
use std::pin::Pin;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::state::{AppConfig, AppState, SubtitleEvent};

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    temperature: f32,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageOwned,
}

#[derive(Debug, Deserialize)]
struct ChatMessageOwned {
    content: String,
}

/// Translation is deliberately kept behind this interface so a local model
/// can replace DeepSeek without changing subtitle delivery or the overlay.
pub trait Translator: Send + Sync {
    fn translate<'a>(
        &'a self,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;
}

#[derive(Clone)]
pub struct DeepSeekTranslator {
    config: AppConfig,
}

impl DeepSeekTranslator {
    pub fn new(config: AppConfig) -> Self {
        Self { config }
    }
}

impl Translator for DeepSeekTranslator {
    fn translate<'a>(
        &'a self,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
        Box::pin(translate_deepseek(&self.config, text))
    }
}

pub async fn translate(config: &AppConfig, text: &str) -> Result<String, String> {
    DeepSeekTranslator::new(config.clone())
        .translate(text)
        .await
}

async fn translate_deepseek(config: &AppConfig, text: &str) -> Result<String, String> {
    if !config.deepseek.enabled {
        return Err("翻译功能已禁用".into());
    }
    if config.deepseek.api_key.trim().is_empty() {
        return Err("尚未配置 DeepSeek API Key".into());
    }
    let base = config.deepseek.base_url.trim().trim_end_matches('/');
    if !(base.starts_with("https://") || base.starts_with("http://")) {
        return Err("DeepSeek API 地址必须以 http:// 或 https:// 开头".into());
    }
    let url = format!("{base}/chat/completions");
    let request = ChatRequest {
        model: config.deepseek.model.trim(),
        messages: vec![
            ChatMessage {
                role: "system",
                content: "你是实时字幕翻译器。将用户提供的中文准确、自然、简洁地翻译成英文。只输出英文翻译，不要解释，不要加引号。",
            },
            ChatMessage {
                role: "user",
                content: text,
            },
        ],
        temperature: 0.2,
        stream: false,
    };
    let client = reqwest::Client::builder()
        .user_agent("voice-caption-studio/0.1.15")
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(url)
        .bearer_auth(&config.deepseek.api_key)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("无法连接 DeepSeek：{error}"))?;
    let status = response.status();
    let body = response.text().await.map_err(|error| error.to_string())?;
    if status != StatusCode::OK {
        return Err(format!(
            "DeepSeek 返回 HTTP {status}：{}",
            truncate(&body, 180)
        ));
    }
    let parsed: ChatResponse =
        serde_json::from_str(&body).map_err(|error| format!("DeepSeek 返回格式异常：{error}"))?;
    parsed
        .choices
        .first()
        .map(|choice| choice.message.content.trim().to_string())
        .filter(|content| !content.is_empty())
        .ok_or_else(|| "DeepSeek 没有返回翻译文本".into())
}

pub async fn publish_with_translation(state: &AppState, mut event: SubtitleEvent) -> SubtitleEvent {
    state.publish_subtitle(event.clone()).await;
    if event.kind != "final" || event.chinese.trim().is_empty() {
        return event;
    }
    let config = state.config.read().await.clone();
    if !config.deepseek.enabled {
        return event;
    }
    match translate(&config, &event.chinese).await {
        Ok(english) => event.english = Some(english),
        Err(error) => event.translation_error = Some(error),
    }
    state.publish_subtitle(event.clone()).await;
    event
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let result: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{result}…")
    } else {
        result
    }
}
