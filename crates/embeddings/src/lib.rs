use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct EmbeddingClient {
    http: Client,
    base_url: String,
    model: String,
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

impl EmbeddingClient {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            http: Client::builder()
                .connect_timeout(Duration::from_secs(read_env_u64(
                    "EMBEDDING_CONNECT_TIMEOUT_SECS",
                    5,
                )))
                .timeout(Duration::from_secs(read_env_u64(
                    "EMBEDDING_REQUEST_TIMEOUT_SECS",
                    15,
                )))
                .pool_idle_timeout(Duration::from_secs(read_env_u64(
                    "EMBEDDING_POOL_IDLE_TIMEOUT_SECS",
                    30,
                )))
                .build()
                .unwrap_or_else(|_| Client::new()),
            base_url: base_url.into(),
            model: model.into(),
        }
    }

    pub fn from_env() -> Self {
        let base_url = std::env::var("LOCAL_EMBEDDING_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8114/v1/embeddings".to_owned());
        let model = std::env::var("LOCAL_EMBEDDING_MODEL")
            .unwrap_or_else(|_| "Qwen/Qwen3-Embedding-0.6B".to_owned());
        Self::new(base_url, model)
    }

    pub async fn embed(&self, input: &str) -> Result<Vec<f32>> {
        let response = self
            .http
            .post(&self.base_url)
            .json(&EmbeddingRequest {
                model: &self.model,
                input,
            })
            .send()
            .await
            .context("embedding request failed")?
            .error_for_status()
            .context("embedding service returned non-success status")?
            .json::<EmbeddingResponse>()
            .await
            .context("failed to parse embedding response")?;

        response
            .data
            .into_iter()
            .next()
            .map(|item| item.embedding)
            .context("embedding response did not include vectors")
    }
}

fn read_env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}
