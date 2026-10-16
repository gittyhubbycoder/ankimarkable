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

    pub async fn models(&self) -> Result<Vec<Model>> {
        let value: Value = self
            .client
            .get(format!("{API}/models"))
            .bearer_auth(&self.key)
            .send()
            .await
            .context("request OpenRouter models")?
            .error_for_status()
            .context("OpenRouter model list returned an error")?
            .json()
            .await
            .context("decode OpenRouter model list")?;
        let list = value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("OpenRouter model list has no data array"))?;
        let mut models: Vec<Model> = list
            .iter()
            .filter_map(|item| {
                let id = item.get("id")?.as_str()?.to_string();
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
        let value: Value = self
            .client
            .post(format!("{API}/chat/completions"))
            .bearer_auth(&self.key)
            .header("X-OpenRouter-Title", "OpenRouter Chat")
            .json(&body)
            .send()
            .await
            .context("send OpenRouter chat request")?
            .error_for_status()
            .context("OpenRouter chat request returned an error")?
            .json()
            .await
            .context("decode OpenRouter chat response")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_type_keeps_api_id_and_display_name() {
        let model = Model {
            id: "vendor/model".into(),
            name: "Model".into(),
        };
        assert_eq!(model.id, "vendor/model");
        assert_eq!(model.name, "Model");
    }
}
