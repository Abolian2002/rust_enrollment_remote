use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use futures::{Stream, StreamExt, stream};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmResponse {
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

pub type LlmDeltaStream = Pin<Box<dyn Stream<Item = Result<String>> + Send>>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, messages: &[LlmMessage]) -> Result<LlmResponse>;

    async fn stream_complete(&self, messages: &[LlmMessage]) -> Result<LlmDeltaStream> {
        let response = self.complete(messages).await?;
        Ok(Box::pin(stream::iter(
            response
                .content
                .split_inclusive(['。', '！', '？', '\n'])
                .filter(|part| !part.is_empty())
                .map(|part| Ok(part.to_owned()))
                .collect::<Vec<_>>(),
        )))
    }
}

#[derive(Clone)]
pub struct OpenAiCompatibleClient {
    http: Client,
    base_url: String,
    api_key: String,
    model: String,
    concurrency: Arc<LlmConcurrencyLimiter>,
}

struct LlmConcurrencyLimiter {
    semaphore: Arc<Semaphore>,
    max_concurrent: usize,
    queue_timeout: Duration,
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: &'a [LlmMessage],
    temperature: f32,
    #[serde(rename = "max_tokens")]
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    enable_thinking: Option<bool>,
    #[serde(default)]
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    content: Option<String>,
}

impl OpenAiCompatibleClient {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            http: Client::builder()
                .connect_timeout(Duration::from_secs(read_env_u64(
                    "LLM_CONNECT_TIMEOUT_SECS",
                    10,
                )))
                .timeout(Duration::from_secs(read_env_u64(
                    "LLM_REQUEST_TIMEOUT_SECS",
                    45,
                )))
                .pool_idle_timeout(Duration::from_secs(read_env_u64(
                    "LLM_POOL_IDLE_TIMEOUT_SECS",
                    30,
                )))
                .build()
                .unwrap_or_else(|_| Client::new()),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key: api_key.into(),
            model: model.into(),
            concurrency: Arc::new(LlmConcurrencyLimiter::from_env()),
        }
    }

    pub fn from_env_for_synthesis() -> Option<Self> {
        let base_url = std::env::var("OPENAI_COMPAT_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "https://dashscope.aliyuncs.com/compatible-mode/v1".to_owned());
        let api_key = std::env::var("OPENAI_COMPAT_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| std::env::var("DASHSCOPE_API_KEY").ok())?;
        let model = std::env::var("OPENAI_SYNTHESIS_MODEL")
            .or_else(|_| std::env::var("OPENAI_AGENT_MODEL"))
            .unwrap_or_else(|_| "qwen3.7-plus".to_owned());
        Some(Self::new(base_url, api_key, model))
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    async fn acquire_llm_permit(&self, operation: &'static str) -> Result<OwnedSemaphorePermit> {
        let started_at = Instant::now();
        let acquire = self.concurrency.semaphore.clone().acquire_owned();
        tokio::pin!(acquire);

        tokio::select! {
            result = &mut acquire => {
                let permit = result.context("llm concurrency limiter closed")?;
                tracing::info!(
                    operation,
                    model = %self.model,
                    queue_wait_ms = started_at.elapsed().as_millis() as u64,
                    available_permits = self.concurrency.semaphore.available_permits(),
                    max_concurrent = self.concurrency.max_concurrent,
                    "llm concurrency permit acquired"
                );
                Ok(permit)
            }
            _ = tokio::time::sleep(self.concurrency.queue_timeout) => {
                tracing::warn!(
                    operation,
                    model = %self.model,
                    queue_timeout_ms = self.concurrency.queue_timeout.as_millis() as u64,
                    "llm concurrency queue wait timed out"
                );
                Err(anyhow!(
                    "LLM 当前排队时间较长，已跳过本轮模型润色。"
                ))
            }
        }
    }
}

fn read_env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn read_env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

impl LlmConcurrencyLimiter {
    fn from_env() -> Self {
        let max_concurrent = read_env_usize("LLM_MAX_CONCURRENT_REQUESTS", 3);
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            max_concurrent,
            queue_timeout: Duration::from_millis(read_env_u64("LLM_QUEUE_TIMEOUT_MS", 20_000)),
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleClient {
    async fn complete(&self, messages: &[LlmMessage]) -> Result<LlmResponse> {
        let _permit = self.acquire_llm_permit("complete").await?;
        let url = format!("{}/chat/completions", self.base_url);
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&ChatCompletionRequest {
                model: &self.model,
                messages,
                temperature: 0.2,
                max_tokens: 1600,
                enable_thinking: read_optional_bool(
                    std::env::var("OPENAI_COMPAT_ENABLE_THINKING")
                        .ok()
                        .as_deref(),
                    std::env::var("DASHSCOPE_ENABLE_THINKING").ok().as_deref(),
                ),
                stream: false,
            })
            .send()
            .await
            .context("llm request failed")?
            .error_for_status()
            .context("llm returned non-success status")?
            .json::<ChatCompletionResponse>()
            .await
            .context("failed to parse llm response")?;

        let content = response
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message.content)
            .unwrap_or_default();

        Ok(LlmResponse {
            content,
            tool_calls: Vec::new(),
        })
    }

    async fn stream_complete(&self, messages: &[LlmMessage]) -> Result<LlmDeltaStream> {
        let permit = self.acquire_llm_permit("stream_complete").await?;
        let url = format!("{}/chat/completions", self.base_url);
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&ChatCompletionRequest {
                model: &self.model,
                messages,
                temperature: 0.2,
                max_tokens: 1600,
                enable_thinking: read_optional_bool(
                    std::env::var("OPENAI_COMPAT_ENABLE_THINKING")
                        .ok()
                        .as_deref(),
                    std::env::var("DASHSCOPE_ENABLE_THINKING").ok().as_deref(),
                ),
                stream: true,
            })
            .send()
            .await
            .context("llm streaming request failed")?
            .error_for_status()
            .context("llm streaming returned non-success status")?;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<String>>(32);
        tokio::spawn(async move {
            let _permit = permit;
            let mut bytes = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(item) = bytes.next().await {
                match item {
                    Ok(chunk) => {
                        buffer.push_str(&String::from_utf8_lossy(&chunk));
                        while let Some(line) = take_sse_line(&mut buffer) {
                            match parse_sse_delta(&line) {
                                SseParseOutcome::Delta(delta) => {
                                    if tx.send(Ok(delta)).await.is_err() {
                                        return;
                                    }
                                }
                                SseParseOutcome::Done => return,
                                SseParseOutcome::Skip => {}
                                SseParseOutcome::Error(error) => {
                                    let _ = tx.send(Err(error)).await;
                                    return;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        let _ = tx
                            .send(Err(anyhow!("llm streaming chunk failed: {error}")))
                            .await;
                        return;
                    }
                }
            }

            if !buffer.trim().is_empty() {
                match parse_sse_delta(buffer.trim()) {
                    SseParseOutcome::Delta(delta) => {
                        let _ = tx.send(Ok(delta)).await;
                    }
                    SseParseOutcome::Error(error) => {
                        let _ = tx.send(Err(error)).await;
                    }
                    SseParseOutcome::Done | SseParseOutcome::Skip => {}
                }
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

enum SseParseOutcome {
    Delta(String),
    Done,
    Skip,
    Error(anyhow::Error),
}

fn take_sse_line(buffer: &mut String) -> Option<String> {
    let newline = buffer.find('\n')?;
    let mut line = buffer[..newline].to_owned();
    if line.ends_with('\r') {
        line.pop();
    }
    buffer.drain(..=newline);
    Some(line)
}

fn parse_sse_delta(line: &str) -> SseParseOutcome {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') {
        return SseParseOutcome::Skip;
    }
    let Some(data) = line.strip_prefix("data:").map(str::trim) else {
        return SseParseOutcome::Skip;
    };
    if data == "[DONE]" {
        return SseParseOutcome::Done;
    }

    let value = match serde_json::from_str::<Value>(data) {
        Ok(value) => value,
        Err(error) => {
            return SseParseOutcome::Error(anyhow!("failed to parse llm stream event: {error}"));
        }
    };

    let Some(content) = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("delta"))
        .and_then(|delta| delta.get("content"))
        .and_then(|content| content.as_str())
    else {
        return SseParseOutcome::Skip;
    };

    if content.is_empty() {
        SseParseOutcome::Skip
    } else {
        SseParseOutcome::Delta(content.to_owned())
    }
}

fn read_optional_bool(first: Option<&str>, second: Option<&str>) -> Option<bool> {
    first.or(second).and_then(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        }
    })
}

#[derive(Debug, Clone)]
pub struct DeterministicLlm {
    reply: String,
}

impl DeterministicLlm {
    pub fn new(reply: impl Into<String>) -> Self {
        Self {
            reply: reply.into(),
        }
    }
}

#[async_trait]
impl LlmProvider for DeterministicLlm {
    async fn complete(&self, _messages: &[LlmMessage]) -> Result<LlmResponse> {
        Ok(LlmResponse {
            content: self.reply.clone(),
            tool_calls: Vec::new(),
        })
    }
}
