use std::future::Future;
use std::pin::Pin;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::state::{AppConfig, AppState, GlossaryEntry, SubtitleEvent};

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
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Debug, Default, Deserialize)]
struct ChatUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageOwned,
}

#[derive(Debug, Deserialize)]
struct ChatMessageOwned {
    content: String,
}

#[derive(Debug, Clone)]
pub struct TranslationResult {
    pub text: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub usage_estimated: bool,
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
        let config = self.config.clone();
        Box::pin(async move {
            translate_with_usage(&config, text)
                .await
                .map(|result| result.text)
        })
    }
}

pub async fn translate(config: &AppConfig, text: &str) -> Result<String, String> {
    translate_with_usage(config, text)
        .await
        .map(|result| result.text)
}

pub async fn translate_with_usage(
    config: &AppConfig,
    text: &str,
) -> Result<TranslationResult, String> {
    if let Some(result) = local_glossary_translation(text, &config.glossary) {
        return Ok(result);
    }
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
    // Replace only terms that occur in this sentence. The complete glossary
    // never enters the prompt, so maintaining a large local glossary does not
    // add a fixed prompt-token cost to every request.
    let translation_input = apply_local_glossary(text, &config.glossary);
    let system_prompt = "你是实时字幕翻译器。将用户提供的中文准确、自然、简洁地翻译成英文。只输出英文翻译，不要解释，不要加引号。";
    let request = ChatRequest {
        model: config.deepseek.model.trim(),
        messages: vec![
            ChatMessage {
                role: "system",
                content: system_prompt,
            },
            ChatMessage {
                role: "user",
                content: &translation_input,
            },
        ],
        temperature: 0.2,
        stream: false,
    };
    let client = reqwest::Client::builder()
        .user_agent("voice-caption-studio/0.1.18")
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
    let translated_text = parsed
        .choices
        .first()
        .map(|choice| choice.message.content.trim().to_string())
        .filter(|content| !content.is_empty())
        .ok_or_else(|| "DeepSeek 没有返回翻译文本".to_string())?;
    let translated_text = normalize_glossary_output(&translated_text, text, &config.glossary);
    let estimated_prompt = estimate_tokens(system_prompt) + estimate_tokens(&translation_input);
    let estimated_completion = estimate_tokens(&translated_text);
    let (prompt_tokens, completion_tokens, total_tokens, usage_estimated) = match parsed.usage {
        Some(usage)
            if usage.prompt_tokens > 0 || usage.completion_tokens > 0 || usage.total_tokens > 0 =>
        {
            let total = if usage.total_tokens > 0 {
                usage.total_tokens
            } else {
                usage.prompt_tokens.saturating_add(usage.completion_tokens)
            };
            (usage.prompt_tokens, usage.completion_tokens, total, false)
        }
        _ => (
            estimated_prompt,
            estimated_completion,
            estimated_prompt.saturating_add(estimated_completion),
            true,
        ),
    };
    Ok(TranslationResult {
        text: translated_text,
        prompt_tokens,
        completion_tokens,
        total_tokens,
        usage_estimated,
    })
}

fn estimate_tokens(text: &str) -> u64 {
    let count = text.chars().count() as u64;
    if count == 0 {
        return 0;
    }
    if text.chars().any(|character| !character.is_ascii()) {
        count
    } else {
        ((count + 3) / 4).max(1)
    }
}

pub async fn publish_with_translation(
    state: &AppState,
    mut event: SubtitleEvent,
) -> (SubtitleEvent, Option<TranslationResult>) {
    state.publish_subtitle(event.clone()).await;
    if event.kind != "final" || event.chinese.trim().is_empty() {
        return (event, None);
    }
    let config = state.config.read().await.clone();
    if !config.deepseek.enabled {
        if let Some(result) = local_glossary_translation(&event.chinese, &config.glossary) {
            event.english = Some(result.text.clone());
            state.publish_subtitle(event.clone()).await;
        }
        return (event, None);
    }
    let result = match translate_with_usage(&config, &event.chinese).await {
        Ok(result) => {
            event.english = Some(result.text.clone());
            Some(result)
        }
        Err(error) => {
            event.translation_error = Some(error);
            None
        }
    };
    state.publish_subtitle(event.clone()).await;
    (event, result)
}

/// Translate a subtitle without calling a remote service when it is an exact
/// glossary entry. This is useful for short fixed labels and costs zero API
/// tokens; ordinary sentences still go through the configured translator.
pub fn local_glossary_translation(
    text: &str,
    glossary: &[GlossaryEntry],
) -> Option<TranslationResult> {
    let source = text.trim();
    glossary
        .iter()
        .find(|entry| entry.source.trim() == source && !entry.target.trim().is_empty())
        .map(|entry| TranslationResult {
            text: entry.target.trim().to_string(),
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            usage_estimated: false,
        })
}

fn apply_local_glossary(text: &str, glossary: &[GlossaryEntry]) -> String {
    let mut entries: Vec<&GlossaryEntry> = glossary
        .iter()
        .filter(|entry| {
            let source = entry.source.trim();
            !source.is_empty() && !entry.target.trim().is_empty() && text.contains(source)
        })
        .collect();
    // Longest first avoids replacing a short term inside a more specific one.
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.source.trim().chars().count()));
    let mut result = text.to_string();
    for entry in entries {
        result = result.replace(entry.source.trim(), entry.target.trim());
    }
    result
}

fn normalize_glossary_output(
    translated: &str,
    source_text: &str,
    glossary: &[GlossaryEntry],
) -> String {
    let mut result = translated.to_string();
    for entry in glossary {
        let source = entry.source.trim();
        let target = entry.target.trim();
        if !source.is_empty() && !target.is_empty() && source_text.contains(source) {
            result = replace_ascii_case_insensitive(&result, target, target);
        }
    }
    result
}

fn replace_ascii_case_insensitive(text: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() || !needle.is_ascii() {
        return text.to_string();
    }
    let lowered = text.to_ascii_lowercase();
    let lowered_needle = needle.to_ascii_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut search_start = 0;
    while let Some(relative_start) = lowered[search_start..].find(&lowered_needle) {
        let start = search_start + relative_start;
        let end = start + needle.len();
        result.push_str(&text[search_start..start]);
        result.push_str(replacement);
        search_start = end;
    }
    result.push_str(&text[search_start..]);
    result
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

#[cfg(test)]
mod tests {
    use super::*;

    fn glossary() -> Vec<GlossaryEntry> {
        vec![GlossaryEntry {
            source: "英伟达广播".into(),
            target: "NVIDIA Broadcast".into(),
        }]
    }

    #[test]
    fn exact_glossary_entry_is_local_and_zero_token() {
        let result = local_glossary_translation("英伟达广播", &glossary()).unwrap();
        assert_eq!(result.text, "NVIDIA Broadcast");
        assert_eq!(result.total_tokens, 0);
        assert!(!result.usage_estimated);
    }

    #[test]
    fn glossary_replaces_only_matched_terms() {
        let result = apply_local_glossary("我打开了英伟达广播", &glossary());
        assert_eq!(result, "我打开了NVIDIA Broadcast");
    }

    #[test]
    fn glossary_normalizes_translated_term_casing() {
        let result = normalize_glossary_output(
            "I enabled nvidia broadcast",
            "我打开了英伟达广播",
            &glossary(),
        );
        assert_eq!(result, "I enabled NVIDIA Broadcast");
    }
}
