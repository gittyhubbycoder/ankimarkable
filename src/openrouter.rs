//! OpenRouter API client for model discovery and multimodal chat.
//!
//! All network methods are async; the UI owns a worker thread/runtime so requests
//! never stall the low-latency pen capture loop.

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use reqwest::Client;
use serde_json::{json, Value};

const API: &str = "https://openrouter.ai/api/v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Clone)]
pub struct OpenRouter {
    client: Client,
    key: String,
}

impl OpenRouter {
    /// Read the secret once from the application environment. Never log it.
    pub fn from_env() -> Result<Self> {
        let key = std::env::var("OPENROUTER_API_KEY").context("OPENROUTER_API_KEY is not set")?;
        Self::with_key(key)
    }

    /// Build a client from an explicit key (used by `from_env` and by tests
    /// that need a controlled bogus key without touching process env).
    pub fn with_key(key: impl Into<String>) -> Result<Self> {
        let key = key.into();
        if key.trim().is_empty() {
            bail!("OPENROUTER_API_KEY is empty");
        }
        Ok(Self {
            client: Client::builder()
                .user_agent("openrouter-chat/0.1")
                .timeout(std::time::Duration::from_secs(90))
                .build()
                .context("create OpenRouter HTTP client")?,
            key,
        })
    }
    /// Masked key hint safe to show on screen, e.g. `sk-or-v1-…abcd`.
    /// The full secret is never exposed; it lives in the launch environment.
    /// Byte slicing is char-boundary safe so unusual keys cannot panic.
    pub fn key_hint(key: &str) -> String {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return "not set".to_string();
        }
        let chars: Vec<char> = trimmed.chars().collect();
        if chars.len() <= 12 {
            return "set (short key)".to_string();
        }
        let head: String = chars[..7].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}…{tail}")
    }

    /// Free-only model catalog. A model counts as free when it is an explicit
    /// `:free` variant, or when both its prompt and completion prices are zero.
    /// Pricing values are decimal strings, so `"0"`, `"0.0"` etc. all count.
    pub async fn models(&self) -> Result<Vec<Model>> {
        let response = self
            .client
            .get(format!("{API}/models"))
            .bearer_auth(&self.key)
            .send()
            .await
            .context("request OpenRouter models: network/TLS failure")?;
        let value: Value = check_status(response, "OpenRouter model list").await?;
        let list = value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("OpenRouter model list has no data array"))?;
        let mut models: Vec<Model> = list
            .iter()
            .filter_map(|item| {
                let id = item.get("id")?.as_str()?.to_string();
                if !is_free_model(item) {
                    return None;
                }
                if item
                    .get("architecture")
                    .and_then(|a| a.get("input_modalities"))
                    .and_then(Value::as_array)
                    .map_or(false, |v| !v.iter().any(|m| m.as_str() == Some("text")))
                {
                    return None;
                }
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_string();
                Some(Model { id, name })
            })
            .collect();
        models.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        models.dedup_by(|a, b| a.id == b.id);
        Ok(models)
    }

    pub async fn complete(&self, model: &str, messages: &[Message]) -> Result<String> {
        let body = json!({
            "model": model,
            "messages": messages.iter().map(|m| json!({"role": m.role, "content": m.content})).collect::<Vec<_>>(),
        });
        self.completion(body).await
    }

    /// OCR-assisted ink path: ask a vision-capable selected model to transcribe
    /// the pad, then submit the transcription as an ordinary text chat turn.
    pub async fn ink_to_text(
        &self,
        model: &str,
        history: &[Message],
        png: &[u8],
    ) -> Result<(String, String)> {
        let transcription = self.transcribe(model, png).await?;
        if transcription.trim().is_empty() {
            bail!("OCR did not find readable text");
        }
        let mut messages = history
            .iter()
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect::<Vec<_>>();
        messages.push(json!({"role":"user", "content":format!("Handwritten prompt (OCR): {}", transcription.trim())}));
        let answer = self
            .completion(json!({"model": model, "messages": messages}))
            .await?;
        Ok((transcription.trim().to_string(), answer))
    }

    /// Direct multimodal path: the PNG is included as a data URL in an image_url
    /// content part, with prior conversational turns retained.
    pub async fn send_image(
        &self,
        model: &str,
        history: &[Message],
        prompt: &str,
        png: &[u8],
    ) -> Result<String> {
        let data_url = format!("data:image/png;base64,{}", STANDARD.encode(png));
        let mut messages = history
            .iter()
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect::<Vec<_>>();
        messages.push(json!({"role":"user", "content":[
            {"type":"text", "text":prompt},
            {"type":"image_url", "image_url":{"url":data_url}}
        ]}));
        self.completion(json!({"model":model, "messages":messages}))
            .await
    }

    async fn transcribe(&self, model: &str, png: &[u8]) -> Result<String> {
        let data_url = format!("data:image/png;base64,{}", STANDARD.encode(png));
        let body = json!({
            "model": model,
            "messages": [{"role":"user", "content":[
                {"type":"text", "text":"Transcribe all handwritten text in this image exactly. Return only the transcription; do not answer it, explain it, or add formatting."},
                {"type":"image_url", "image_url":{"url":data_url}}
            ]}]
        });
        self.completion(body).await
    }

    async fn completion(&self, body: Value) -> Result<String> {
        let response = self
            .client
            .post(format!("{API}/chat/completions"))
            .bearer_auth(&self.key)
            .header("X-OpenRouter-Title", "OpenRouter Chat")
            .json(&body)
            .send()
            .await
            .context("send OpenRouter chat request: network/TLS failure")?;
        let value: Value = check_status(response, "OpenRouter chat request").await?;
        let content = &value["choices"][0]["message"]["content"];
        if let Some(text) = content.as_str() {
            return Ok(text.to_string());
        }
        if let Some(parts) = content.as_array() {
            return Ok(parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"));
        }
        bail!("OpenRouter returned no assistant message content")
    }
}

fn is_free_model(item: &Value) -> bool {
    if item
        .get("id")
        .and_then(Value::as_str)
        .map_or(false, |id| id.ends_with(":free"))
    {
        return true;
    }
    let price = |key: &str| {
        item.get("pricing")
            .and_then(|p| p.get(key))
            .and_then(Value::as_str)
            .unwrap_or("")
    };
    is_zero_price(price("prompt")) && is_zero_price(price("completion"))
}

fn is_zero_price(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.parse::<f64>().map_or(false, |v| v == 0.0)
}

/// Read the body once and report HTTP failures with status + a body snippet,
/// so device-side errors (401/404/429/5xx) are distinguishable from TLS or
/// network failures. Success responses are parsed as JSON.
async fn check_status(response: reqwest::Response, context: &str) -> Result<Value> {
    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| format!("{context}: read response body"))?;
    if !status.is_success() {
        bail!("{}: HTTP {}: {}", context, status, snippet(&text, 500));
    }
    serde_json::from_str(&text).with_context(|| format!("{context}: decode JSON response"))
}

/// First `limit` chars of a body with whitespace collapsed, for error lines.
/// Char-boundary safe: API bodies may contain multibyte text, and slicing
/// mid-character would panic the network worker and wedge the UI busy state.
pub fn snippet(text: &str, limit: usize) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = collapsed.chars().collect();
    if chars.len() <= limit {
        collapsed
    } else {
        let head: String = chars[..limit].iter().collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_type_keeps_api_id_and_display_name() {
        let model = Model {
            id: "vendor/model".into(),
            name: "Model".into(),
        };
        assert_eq!(model.id, "vendor/model");
        assert_eq!(model.name, "Model");
    }

    #[test]
    fn free_filter_keeps_free_variant_and_zero_pricing() {
        let free_suffix =
            json!({"id": "x/y:free", "pricing": {"prompt": "0.1", "completion": "0.1"}});
        let zero_price = json!({"id": "x/y", "pricing": {"prompt": "0", "completion": "0"}});
        let paid = json!({"id": "x/y", "pricing": {"prompt": "0", "completion": "0.01"}});
        let no_pricing = json!({"id": "x/y"});
        assert!(is_free_model(&free_suffix));
        assert!(is_free_model(&zero_price));
        assert!(!is_free_model(&paid));
        assert!(!is_free_model(&no_pricing));
    }

    #[test]
    fn key_hint_masks_but_confirms_presence() {
        assert_eq!(OpenRouter::key_hint(""), "not set");
        assert_eq!(OpenRouter::key_hint("sk-or-v1-abcdef1234"), "sk-or-v…1234");
    }

    #[test]
    fn error_snippet_collapses_and_truncates() {
        let s = snippet("  a\n  b   c  ", 100);
        assert_eq!(s, "a b c");
        let long = "x".repeat(600);
        assert!(snippet(&long, 500).ends_with("…"));
    }
}
