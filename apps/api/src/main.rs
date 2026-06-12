use admissions_agent::AdmissionsAgent;
use axum::{
    Json, Router,
    body::Body,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, Sse},
    },
    routing::{get, patch, post, put},
};
use db::Database;
use domain::{ChatReply, ChatRequest, fail, ok, ok_with_meta};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::{
    convert::Infallible,
    fmt,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore, mpsc, watch};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    db: Database,
    agent: AdmissionsAgent,
    chat_semaphore: Arc<Semaphore>,
    tts_limiter: Arc<TtsTokenLimiter>,
    tts_http: reqwest::Client,
    voice_tts_semaphore: Arc<Semaphore>,
    active_voice_sessions: Arc<Mutex<HashMap<String, VoiceSessionCancel>>>,
    next_voice_session_id: Arc<AtomicU64>,
}

#[derive(Clone)]
struct VoiceSessionCancel {
    id: u64,
    cancel_tx: watch::Sender<bool>,
}

struct TtsRateBucket {
    window_started_at: Instant,
    count: u32,
}

struct TtsTokenLimiter {
    limit_per_minute: u32,
    buckets: Mutex<HashMap<String, TtsRateBucket>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TtsSpeechRequest {
    input: String,
    model: Option<String>,
    voice: Option<String>,
}

#[derive(Debug)]
enum VoiceWsOut {
    Json(serde_json::Value),
    Audio(Vec<u8>),
}

#[derive(Debug, Clone, Copy)]
struct ServerTtsSegmenterConfig {
    first_min_chars: usize,
    first_max_chars: usize,
    min_chars: usize,
    max_chars: usize,
    flush_after: Duration,
}

#[derive(Debug)]
struct ServerTtsSegmenter {
    buffer: String,
    emitted_count: usize,
    config: ServerTtsSegmenterConfig,
}

#[derive(Debug)]
struct TtsSegmentStreamError {
    message: String,
    audio_started: bool,
    client_disconnected: bool,
    queue_timeout: bool,
}

impl TtsSegmentStreamError {
    fn upstream(message: impl Into<String>, audio_started: bool) -> Self {
        Self {
            message: message.into(),
            audio_started,
            client_disconnected: false,
            queue_timeout: false,
        }
    }

    fn queue_timeout(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            audio_started: false,
            client_disconnected: false,
            queue_timeout: true,
        }
    }

    fn client_disconnected(audio_started: bool) -> Self {
        Self {
            message: "voice websocket client disconnected".to_owned(),
            audio_started,
            client_disconnected: true,
            queue_timeout: false,
        }
    }
}

#[derive(Debug)]
enum VoiceTtsQueueWaitError {
    Cancelled,
    Closed,
    TimedOut,
}

impl fmt::Display for VoiceTtsQueueWaitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VoiceTtsQueueWaitError::Cancelled => f.write_str("voice TTS queue wait cancelled"),
            VoiceTtsQueueWaitError::Closed => f.write_str("voice TTS concurrency limiter closed"),
            VoiceTtsQueueWaitError::TimedOut => {
                f.write_str("语音合成排队时间较长，已跳过本段语音。")
            }
        }
    }
}

impl fmt::Display for TtsSegmentStreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TtsSegmentStreamError {}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceChatErrorPayload {
    event: &'static str,
    code: &'static str,
    message: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminFeedbackRequest {
    conversation_id: Option<String>,
    message_id: Option<String>,
    feedback_type: String,
    comment: Option<String>,
    handled_by: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminTicketUpdateRequest {
    status: Option<String>,
    resolution: Option<String>,
    handled_by: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminSettingsUpdateRequest {
    welcome_message: String,
    fallback_message: String,
    updated_by: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminFaqWriteRequest {
    question: Option<String>,
    answer: Option<String>,
    category: Option<String>,
    tags: Option<Vec<String>>,
    status: Option<String>,
    source_label: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_env();
    init_tracing();
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://postgres:postgres@localhost:55432/hnu_enrollment".to_owned()
    });
    let db = Database::connect_lazy(&database_url)?;
    let state = Arc::new(AppState {
        agent: AdmissionsAgent::new(db.clone()),
        db,
        chat_semaphore: Arc::new(Semaphore::new(read_env_usize(
            "CHAT_MAX_CONCURRENT_REQUESTS",
            40,
        ))),
        tts_limiter: Arc::new(TtsTokenLimiter::new(read_env_u32(
            "TTS_TOKEN_RATE_LIMIT_PER_MINUTE",
            20,
        ))),
        tts_http: build_tts_http_client(),
        voice_tts_semaphore: Arc::new(Semaphore::new(server_tts_max_concurrent_synth())),
        active_voice_sessions: Arc::new(Mutex::new(HashMap::new())),
        next_voice_session_id: Arc::new(AtomicU64::new(1)),
    });
    let app = build_router(state);
    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(4000);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!(%addr, "rust enrollment api listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn load_env() {
    let _ = dotenvy::from_filename(".env");
    let _ = dotenvy::from_filename("../../.env");
}

fn init_tracing() {
    let filter = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| "api=info,admissions_agent=info,tower_http=info".to_owned());
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(filter))
        .with(tracing_subscriber::fmt::layer().json())
        .init();
}

fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/chat", post(chat))
        .route("/api/v1/chat/stream", post(chat_stream))
        .route("/api/v1/chat/voice", get(chat_voice_ws))
        .route("/api/v1/chat/history/{conversation_id}", get(chat_history))
        .route("/api/v1/majors", get(list_majors))
        .route("/api/v1/majors/{slug}", get(get_major))
        .route("/api/v1/admission/scores", get(admission_scores))
        .route(
            "/api/v1/admission/plans/by-major",
            get(admission_plans_by_major),
        )
        .route("/api/v1/knowledge/faq", get(knowledge_faq))
        .route("/api/v1/knowledge/policies", get(knowledge_policies))
        .route("/api/v1/admin/dashboard/summary", get(admin_dashboard))
        .route("/api/v1/admin/analytics/insights", get(admin_insights))
        .route("/api/v1/admin/analytics/special", get(admin_special))
        .route(
            "/api/v1/admin/analytics/admissions",
            get(admin_admissions_analytics),
        )
        .route(
            "/api/v1/admin/analytics/knowledge",
            get(admin_knowledge_coverage),
        )
        .route("/api/v1/admin/analytics/big-screen", get(admin_big_screen))
        .route("/api/v1/admin/conversations", get(admin_conversations))
        .route(
            "/api/v1/admin/conversations/{conversation_id}",
            get(admin_conversation_detail),
        )
        .route("/api/v1/admin/knowledge/faqs", get(admin_knowledge_faqs))
        .route("/api/v1/admin/knowledge/faqs", post(admin_create_faq))
        .route("/api/v1/admin/knowledge/faqs/{faq_id}", patch(admin_update_faq))
        .route(
            "/api/v1/admin/knowledge/chunks",
            get(admin_knowledge_chunks),
        )
        .route("/api/v1/admin/faq", get(admin_knowledge_faqs))
        .route("/api/v1/admin/faq", post(admin_create_faq))
        .route("/api/v1/admin/faq/{faq_id}", put(admin_update_faq))
        .route("/api/v1/admin/faq/{faq_id}", patch(admin_update_faq))
        .route("/api/v1/admin/feedback", post(admin_create_feedback))
        .route("/api/v1/admin/tickets", get(admin_tickets))
        .route("/api/v1/admin/tickets/{ticket_id}", patch(admin_update_ticket))
        .route("/api/v1/admin/settings", get(admin_settings))
        .route("/api/v1/admin/settings", patch(admin_update_settings))
        .route("/api/v1/admin/audit-logs", get(admin_audit_logs))
        .route("/api/v1/tts/token", post(tts_token))
        .route("/api/v1/tts/speech", post(tts_speech))
        .route("/api/v1/tts/stream", post(tts_stream))
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(90),
        ))
        .layer(RequestBodyLimitLayer::new(1024 * 1024))
        .layer(cors_layer())
        .with_state(state)
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db_status = match state.db.health_check().await {
        Ok(()) => "ok",
        Err(_) => "unavailable",
    };
    Json(ok(json!({
        "service": "rust-enrollment-api",
        "status": "ok",
        "database": db_status
    })))
}

fn public_chat_reply(mut reply: ChatReply) -> (ChatReply, Value) {
    if client_diagnostics_enabled() {
        let meta = reply
            .diagnostics
            .as_ref()
            .map(|diagnostics| json!({ "diagnostics": diagnostics }))
            .unwrap_or_else(|| json!({}));
        (reply, meta)
    } else {
        reply.diagnostics = None;
        (reply, json!({}))
    }
}

async fn chat(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ChatRequest>,
) -> impl IntoResponse {
    let Ok(_permit) = state.chat_semaphore.clone().try_acquire_owned() else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(fail("CHAT_BUSY", "当前咨询人数较多，请稍后再试。")),
        )
            .into_response();
    };

    match tokio::time::timeout(agent_timeout_duration(), state.agent.chat(payload)).await {
        Ok(Ok(reply)) => {
            let (reply, meta) = public_chat_reply(reply);
            (StatusCode::OK, Json(ok_with_meta(reply, meta))).into_response()
        }
        Ok(Err(error)) => {
            tracing::error!(error = %error, "chat request failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail(
                    "CHAT_ERROR",
                    "当前咨询人数较多，暂时无法完成本次查询，请稍后再试。",
                )),
            )
                .into_response()
        }
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(fail("CHAT_TIMEOUT", "本次查询耗时较长，请稍后重试。")),
        )
            .into_response(),
    }
}

async fn chat_history(
    State(state): State<Arc<AppState>>,
    Path(conversation_id): Path<String>,
) -> impl IntoResponse {
    if !is_valid_conversation_id(&conversation_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(fail("BAD_REQUEST", "Invalid conversation id")),
        )
            .into_response();
    }
    match state.db.get_conversation_history(&conversation_id).await {
        Ok(reply) => {
            let Some(history) = reply else {
                return (
                    StatusCode::NOT_FOUND,
                    Json(fail("NOT_FOUND", "Conversation not found")),
                )
                    .into_response();
            };
            (StatusCode::OK, Json(ok(history))).into_response()
        }
        Err(error) => {
            tracing::error!(error = %error, "failed to load conversation history");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("HISTORY_ERROR", "无法读取对话历史。")),
            )
                .into_response()
        }
    }
}

async fn chat_stream(
    State(state): State<Arc<AppState>>,
    _headers: HeaderMap,
    Json(payload): Json<ChatRequest>,
) -> Response {
    let Ok(permit) = state.chat_semaphore.clone().try_acquire_owned() else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(fail("CHAT_BUSY", "当前咨询人数较多，请稍后再试。")),
        )
            .into_response();
    };

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(32);
    let agent = state.agent.clone();

    tokio::spawn(async move {
        let _permit = permit;
        let send = |tx: mpsc::Sender<Result<Event, Infallible>>, event: Event| async move {
            tx.send(Ok(event)).await.is_ok()
        };

        if !send(tx.clone(), status_event("resolving")).await {
            return;
        }
        if !send(tx.clone(), status_event("retrieving")).await {
            return;
        }

        let mut generating_sent = false;
        let stream_future = agent.chat_stream_with_deltas(payload, |conversation_id, delta| {
            let tx = tx.clone();
            let should_send_generating = !generating_sent;
            generating_sent = true;
            async move {
                if should_send_generating && !send(tx.clone(), status_event("generating")).await {
                    return false;
                }
                let event = Event::default().event("chunk").data(
                    json!({
                        "conversationId": conversation_id,
                        "delta": delta
                    })
                    .to_string(),
                );
                send(tx, event).await
            }
        });

        match tokio::time::timeout(agent_timeout_duration(), stream_future).await {
            Ok(Ok(reply)) => {
                if !generating_sent && !send(tx.clone(), status_event("generating")).await {
                    return;
                }

                let (reply, meta) = public_chat_reply(reply);
                let event = Event::default().event("message").data(
                    serde_json::to_string(&ok_with_meta(reply, meta))
                        .unwrap_or_else(|_| "{}".to_owned()),
                );
                if !send(tx.clone(), event).await {
                    return;
                }
            }
            Ok(Err(error)) => {
                tracing::error!(error = %error, "stream chat request failed");
                let event = Event::default().event("message").data(
                    serde_json::to_string(&fail(
                        "CHAT_ERROR",
                        "当前咨询人数较多，暂时无法完成本次查询，请稍后再试。",
                    ))
                    .unwrap_or_else(|_| "{}".to_owned()),
                );
                if !send(tx.clone(), event).await {
                    return;
                }
            }
            Err(_) => {
                tracing::warn!("stream chat request timed out");
                let event = Event::default().event("message").data(
                    serde_json::to_string(&fail("CHAT_TIMEOUT", "本次查询耗时较长，请稍后重试。"))
                        .unwrap_or_else(|_| "{}".to_owned()),
                );
                if !send(tx.clone(), event).await {
                    return;
                }
            }
        }

        let _ = send(
            tx,
            Event::default()
                .event("done")
                .data(json!({ "done": true }).to_string()),
        )
        .await;
    });

    Sse::new(ReceiverStream::new(rx))
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

async fn chat_voice_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    if !tts_auth_allowed(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(fail("UNAUTHORIZED", "Voice chat access is not authorized")),
        )
            .into_response();
    }
    let rate_key = client_rate_key(&headers);
    if !state.tts_limiter.allow(&rate_key).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(fail(
                "TTS_RATE_LIMITED",
                "语音服务请求过于频繁，请稍后再试。",
            )),
        )
            .into_response();
    }

    ws.on_upgrade(move |socket| handle_voice_socket(socket, state))
        .into_response()
}

async fn handle_voice_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let payload = match read_voice_chat_init(&mut socket).await {
        Ok(payload) => payload,
        Err(error) => {
            let _ = socket
                .send(Message::Text(
                    json!(VoiceChatErrorPayload {
                        event: "error",
                        code: "VOICE_INIT_ERROR",
                        message: error,
                    })
                    .to_string()
                    .into(),
                ))
                .await;
            let _ = socket.close().await;
            return;
        }
    };

    let Ok(permit) = state.chat_semaphore.clone().try_acquire_owned() else {
        let _ = socket
            .send(Message::Text(
                json!(VoiceChatErrorPayload {
                    event: "error",
                    code: "CHAT_BUSY",
                    message: "当前咨询人数较多，请稍后再试。",
                })
                .to_string()
                .into(),
            ))
            .await;
        let _ = socket.close().await;
        return;
    };

    let (mut sender, mut receiver) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<VoiceWsOut>(64);
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let voice_session_key = payload.conversation_id.clone();
    let voice_session_id = state.next_voice_session_id.fetch_add(1, Ordering::Relaxed);
    if let Some(key) = voice_session_key.as_deref().filter(|key| !key.is_empty()) {
        let previous = state.active_voice_sessions.lock().await.insert(
            key.to_owned(),
            VoiceSessionCancel {
                id: voice_session_id,
                cancel_tx: cancel_tx.clone(),
            },
        );
        if let Some(previous) = previous {
            let _ = previous.cancel_tx.send(true);
            tracing::info!(
                conversation_id = key,
                previous_session_id = previous.id,
                voice_session_id,
                "cancelled previous active voice session for conversation"
            );
        }
    }

    let writer_cancel = cancel_tx.clone();
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            let result = match message {
                VoiceWsOut::Json(value) => {
                    sender.send(Message::Text(value.to_string().into())).await
                }
                VoiceWsOut::Audio(bytes) => sender.send(Message::Binary(bytes.into())).await,
            };
            if result.is_err() {
                let _ = writer_cancel.send(true);
                break;
            }
        }
        let _ = writer_cancel.send(true);
    });

    let reader_cancel = cancel_tx.clone();
    let reader = tokio::spawn(async move {
        while let Some(message) = receiver.next().await {
            match message {
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
        let _ = reader_cancel.send(true);
    });

    let (tts_delta_tx, tts_delta_rx) = mpsc::unbounded_channel::<String>();
    let (tts_segment_tx, tts_segment_rx) = mpsc::channel::<String>(16);
    let segmenter_config = server_tts_segmenter_config();
    let segmenter_cancel = cancel_rx.clone();
    let segmenter = tokio::spawn(run_server_tts_segmenter(
        tts_delta_rx,
        tts_segment_tx,
        segmenter_config,
        segmenter_cancel,
    ));
    let synth = tokio::spawn(run_server_tts_synth(
        state.clone(),
        tts_segment_rx,
        out_tx.clone(),
        cancel_rx.clone(),
        voice_session_key.clone(),
        voice_session_id,
    ));

    let _permit = permit;
    let agent = state.agent.clone();
    let _ = send_voice_json(&out_tx, json!({ "event": "status", "status": "resolving" })).await;
    let _ = send_voice_json(
        &out_tx,
        json!({ "event": "status", "status": "retrieving" }),
    )
    .await;

    let mut generating_sent = false;
    let spoken_tts_text = Arc::new(std::sync::Mutex::new(String::new()));
    let agent_result = {
        let stream_future = agent.chat_stream_with_deltas(payload, |conversation_id, delta| {
            let out_tx = out_tx.clone();
            let tts_delta_tx = tts_delta_tx.clone();
            let cancel_rx = cancel_rx.clone();
            let spoken_tts_text = spoken_tts_text.clone();
            let should_send_generating = !generating_sent;
            generating_sent = true;
            async move {
                if *cancel_rx.borrow() {
                    return false;
                }
                if should_send_generating
                    && !send_voice_json(
                        &out_tx,
                        json!({ "event": "status", "status": "generating" }),
                    )
                    .await
                {
                    return false;
                }
                if !send_voice_json(
                    &out_tx,
                    json!({
                        "event": "chunk",
                        "conversationId": conversation_id,
                        "delta": delta,
                    }),
                )
                .await
                {
                    return false;
                }
                if tts_delta_tx.send(delta.clone()).is_ok() {
                    record_spoken_tts_delta(&spoken_tts_text, &delta);
                }
                true
            }
        });

        tokio::pin!(stream_future);
        let mut agent_cancel_rx = cancel_rx.clone();
        tokio::select! {
            biased;
            changed = agent_cancel_rx.changed() => {
                if changed.is_ok() && *agent_cancel_rx.borrow() {
                    tracing::debug!("voice chat stream cancelled after client disconnect");
                }
                None
            }
            result = tokio::time::timeout(agent_timeout_duration(), &mut stream_future) => Some(result),
        }
    };

    let mut should_cancel_tts = false;
    match agent_result {
        Some(Ok(Ok(reply))) => {
            if !generating_sent {
                let _ = send_voice_json(
                    &out_tx,
                    json!({ "event": "status", "status": "generating" }),
                )
                .await;
            }
            if let Some(suffix) = unspoken_tts_suffix(&spoken_tts_text, &reply.reply) {
                tracing::info!(
                    voice_session_id,
                    suffix_chars = suffix.chars().count(),
                    "server-side voice queued unspoken final reply suffix"
                );
                if tts_delta_tx.send(suffix.clone()).is_ok() {
                    record_spoken_tts_delta(&spoken_tts_text, &suffix);
                }
            }
            let (reply, meta) = public_chat_reply(reply);
            let _ = send_voice_json(
                &out_tx,
                json!({
                    "event": "message",
                    "payload": ok_with_meta(reply, meta),
                }),
            )
            .await;
        }
        Some(Ok(Err(error))) => {
            should_cancel_tts = true;
            tracing::error!(error = %error, "voice chat request failed");
            let _ = send_voice_json(
                &out_tx,
                json!(VoiceChatErrorPayload {
                    event: "error",
                    code: "CHAT_ERROR",
                    message: "当前咨询人数较多，暂时无法完成本次查询，请稍后再试。",
                }),
            )
            .await;
        }
        Some(Err(_)) => {
            should_cancel_tts = true;
            tracing::warn!("voice chat request timed out");
            let _ = send_voice_json(
                &out_tx,
                json!(VoiceChatErrorPayload {
                    event: "error",
                    code: "CHAT_TIMEOUT",
                    message: "本次查询耗时较长，请稍后重试。",
                }),
            )
            .await;
        }
        None => {
            should_cancel_tts = true;
        }
    }
    drop(tts_delta_tx);

    let tts_cancelled = if should_cancel_tts {
        let _ = cancel_tx.send(true);
        abort_voice_task(segmenter);
        abort_voice_task(synth);
        true
    } else {
        wait_voice_tasks_or_cancel(segmenter, synth, cancel_rx.clone()).await
    };
    let _ = send_voice_json(&out_tx, json!({ "event": "done" })).await;
    if !tts_cancelled {
        let _ = cancel_tx.send(true);
    }
    drop(out_tx);
    let _ = writer.await;
    reader.abort();
    if let Some(key) = voice_session_key.as_deref().filter(|key| !key.is_empty()) {
        let mut sessions = state.active_voice_sessions.lock().await;
        if sessions
            .get(key)
            .map(|session| session.id == voice_session_id)
            .unwrap_or(false)
        {
            sessions.remove(key);
        }
    }
}

async fn read_voice_chat_init(socket: &mut WebSocket) -> Result<ChatRequest, &'static str> {
    let message = tokio::time::timeout(Duration::from_secs(10), socket.recv())
        .await
        .map_err(|_| "语音会话初始化超时，请重新发送问题。")?
        .ok_or("语音连接已断开，请重新发送问题。")?
        .map_err(|_| "语音连接异常，请重新发送问题。")?;

    let text = match message {
        Message::Text(text) => text.to_string(),
        Message::Binary(bytes) => {
            String::from_utf8(bytes.to_vec()).map_err(|_| "语音会话初始化数据格式不正确。")?
        }
        _ => return Err("语音会话初始化数据格式不正确。"),
    };

    serde_json::from_str::<ChatRequest>(&text).map_err(|_| "语音会话请求格式不正确。")
}

async fn send_voice_json(out_tx: &mpsc::Sender<VoiceWsOut>, value: serde_json::Value) -> bool {
    out_tx.send(VoiceWsOut::Json(value)).await.is_ok()
}

fn abort_voice_task(task: JoinHandle<()>) {
    if !task.is_finished() {
        task.abort();
    }
}

async fn wait_voice_tasks_or_cancel(
    mut segmenter: JoinHandle<()>,
    mut synth: JoinHandle<()>,
    mut cancel_rx: watch::Receiver<bool>,
) -> bool {
    let mut segmenter_done = false;
    let mut synth_done = false;

    loop {
        if segmenter_done && synth_done {
            return false;
        }

        tokio::select! {
            biased;
            changed = cancel_rx.changed() => {
                if changed.is_ok() && *cancel_rx.borrow() {
                    tracing::info!("server-side voice tasks aborted after cancellation");
                    abort_voice_task(segmenter);
                    abort_voice_task(synth);
                    return true;
                }
                tracing::info!("server-side voice tasks aborted after cancellation channel closed");
                abort_voice_task(segmenter);
                abort_voice_task(synth);
                return true;
            }
            result = &mut segmenter, if !segmenter_done => {
                segmenter_done = true;
                if let Err(error) = result
                    && !error.is_cancelled()
                {
                    tracing::warn!(error = %error, "server-side voice segmenter task failed");
                }
            }
            result = &mut synth, if !synth_done => {
                synth_done = true;
                if let Err(error) = result
                    && !error.is_cancelled()
                {
                    tracing::warn!(error = %error, "server-side voice synth task failed");
                }
            }
        }
    }
}

async fn run_server_tts_segmenter(
    mut delta_rx: mpsc::UnboundedReceiver<String>,
    segment_tx: mpsc::Sender<String>,
    config: ServerTtsSegmenterConfig,
    mut cancel_rx: watch::Receiver<bool>,
) {
    let mut segmenter = ServerTtsSegmenter::new(config);
    loop {
        if *cancel_rx.borrow() {
            return;
        }

        if segmenter.is_empty() {
            match tokio::select! {
                biased;
                changed = cancel_rx.changed() => {
                    if changed.is_ok() && *cancel_rx.borrow() {
                        return;
                    }
                    None
                }
                delta = delta_rx.recv() => delta,
            } {
                Some(delta) => {
                    for segment in segmenter.push(&delta) {
                        if segment_tx.send(segment).await.is_err() {
                            return;
                        }
                    }
                }
                None => break,
            }
            continue;
        }

        match tokio::select! {
            biased;
            changed = cancel_rx.changed() => {
                if changed.is_ok() && *cancel_rx.borrow() {
                    return;
                }
                SegmenterReceive::Closed
            }
            delta = delta_rx.recv() => match delta {
                Some(delta) => SegmenterReceive::Delta(delta),
                None => SegmenterReceive::Closed,
            },
            _ = tokio::time::sleep(config.flush_after) => SegmenterReceive::Flush,
        } {
            SegmenterReceive::Delta(delta) => {
                for segment in segmenter.push(&delta) {
                    if segment_tx.send(segment).await.is_err() {
                        return;
                    }
                }
            }
            SegmenterReceive::Closed => break,
            SegmenterReceive::Flush => {
                if let Some(segment) = segmenter.flush_latency() {
                    if segment_tx.send(segment).await.is_err() {
                        return;
                    }
                }
            }
        }
    }

    for segment in segmenter.finish() {
        if segment_tx.send(segment).await.is_err() {
            return;
        }
    }
}

enum SegmenterReceive {
    Delta(String),
    Flush,
    Closed,
}

async fn run_server_tts_synth(
    state: Arc<AppState>,
    mut segment_rx: mpsc::Receiver<String>,
    out_tx: mpsc::Sender<VoiceWsOut>,
    mut cancel_rx: watch::Receiver<bool>,
    conversation_id: Option<String>,
    voice_session_id: u64,
) {
    let max_retries = server_tts_segment_retries();
    let max_consecutive_failures = server_tts_max_consecutive_failures();
    let retry_delay = server_tts_retry_delay();
    let mut consecutive_failures = 0usize;
    let mut reported_instability = false;
    let mut segment_cancel_rx = cancel_rx.clone();
    let mut next_segment_index = 0usize;

    loop {
        if *cancel_rx.borrow() {
            break;
        }

        let Some(segment) = (tokio::select! {
            biased;
            changed = cancel_rx.changed() => {
                if changed.is_ok() && *cancel_rx.borrow() {
                    break;
                }
                None
            }
            segment = segment_rx.recv() => segment,
        }) else {
            break;
        };

        let segment_index = next_segment_index;
        next_segment_index += 1;
        let mut attempt = 0usize;
        loop {
            match stream_tts_segment(
                &state,
                &segment,
                &out_tx,
                &mut segment_cancel_rx,
                conversation_id.as_deref(),
                voice_session_id,
                segment_index,
            )
            .await
            {
                Ok(()) => {
                    consecutive_failures = 0;
                    break;
                }
                Err(error) if *cancel_rx.borrow() || error.client_disconnected => {
                    tracing::debug!(error = %error, "server-side voice TTS stopped after client disconnect");
                    return;
                }
                Err(error)
                    if !error.audio_started && !error.queue_timeout && attempt < max_retries =>
                {
                    attempt += 1;
                    tracing::warn!(
                        error = %error,
                        attempt,
                        max_retries,
                        segment_index,
                        segment_chars = segment.chars().count(),
                        "server-side voice TTS segment failed before audio; retrying"
                    );
                    tokio::select! {
                        biased;
                        changed = cancel_rx.changed() => {
                            if changed.is_ok() && *cancel_rx.borrow() {
                                return;
                            }
                            return;
                        }
                        _ = tokio::time::sleep(retry_delay) => {}
                    }
                    continue;
                }
                Err(error) => {
                    consecutive_failures += 1;
                    tracing::warn!(
                        error = %error,
                        audio_started = error.audio_started,
                        consecutive_failures,
                        max_consecutive_failures,
                        segment_index,
                        segment_chars = segment.chars().count(),
                        "server-side voice TTS segment dropped"
                    );
                    if !reported_instability {
                        reported_instability = true;
                        let _ = send_voice_json(
                            &out_tx,
                            json!(VoiceChatErrorPayload {
                                event: "tts_error",
                                code: "TTS_STREAM_ERROR",
                                message: "语音播报暂时不稳定，文字回答仍可正常查看。",
                            }),
                        )
                        .await;
                    }
                    if consecutive_failures >= max_consecutive_failures {
                        tracing::warn!(
                            consecutive_failures,
                            "server-side voice TTS stopped after repeated segment failures"
                        );
                        return;
                    }
                    break;
                }
            }
        }
    }
}

async fn stream_tts_segment(
    state: &AppState,
    segment: &str,
    out_tx: &mpsc::Sender<VoiceWsOut>,
    cancel_rx: &mut watch::Receiver<bool>,
    conversation_id: Option<&str>,
    voice_session_id: u64,
    segment_index: usize,
) -> Result<(), TtsSegmentStreamError> {
    if *cancel_rx.borrow() {
        return Err(TtsSegmentStreamError::client_disconnected(false));
    }

    let queue_started_at = Instant::now();
    let _permit = acquire_voice_tts_permit(
        state,
        cancel_rx,
        conversation_id,
        voice_session_id,
        segment_index,
    )
    .await
    .map_err(|error| match error {
        VoiceTtsQueueWaitError::TimedOut => TtsSegmentStreamError::queue_timeout(error.to_string()),
        _ => TtsSegmentStreamError::upstream(error.to_string(), false),
    })?;
    let queue_wait = queue_started_at.elapsed();
    let endpoint = local_tts_stream_url();
    let started_at = Instant::now();
    tracing::info!(
        conversation_id,
        voice_session_id,
        segment_index,
        is_first_segment = segment_index == 0,
        segment_chars = segment.chars().count(),
        queue_wait_ms = queue_wait.as_millis() as u64,
        segment_preview = %tts_log_preview_for_env(segment),
        "server-side voice TTS segment started"
    );
    let request = state
        .tts_http
        .post(endpoint)
        .json(&json!({
            "model": local_tts_model(),
            "voice": local_tts_voice(),
            "input": segment,
        }))
        .send();

    let response = tokio::select! {
        biased;
        changed = cancel_rx.changed() => {
            if changed.is_ok() && *cancel_rx.borrow() {
                return Err(TtsSegmentStreamError::client_disconnected(false));
            }
            return Err(TtsSegmentStreamError::client_disconnected(false));
        }
        result = request => {
            result.map_err(|error| {
                TtsSegmentStreamError::upstream(
                    format!("failed to request local streaming TTS: {error}"),
                    false,
                )
            })?
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text();
        let body_len = tokio::select! {
            biased;
            changed = cancel_rx.changed() => {
                if changed.is_ok() && *cancel_rx.borrow() {
                    return Err(TtsSegmentStreamError::client_disconnected(false));
                }
                return Err(TtsSegmentStreamError::client_disconnected(false));
            }
            result = body => result.map(|body| body.len()).unwrap_or_default(),
        };
        return Err(TtsSegmentStreamError::upstream(
            format!("local streaming TTS returned {status}; body_len={body_len}"),
            false,
        ));
    }

    let mut stream = response.bytes_stream();
    let mut audio_started = false;
    let mut first_audio_ms = None;
    let mut audio_chunks = 0usize;
    let mut audio_bytes = 0usize;
    loop {
        let Some(chunk) = (tokio::select! {
            biased;
            changed = cancel_rx.changed() => {
                if changed.is_ok() && *cancel_rx.borrow() {
                    return Err(TtsSegmentStreamError::client_disconnected(audio_started));
                }
                return Err(TtsSegmentStreamError::client_disconnected(audio_started));
            }
            chunk = stream.next() => chunk,
        }) else {
            break;
        };
        let chunk = chunk.map_err(|error| {
            TtsSegmentStreamError::upstream(
                format!("error decoding streaming TTS response body: {error}"),
                audio_started,
            )
        })?;
        if !chunk.is_empty()
            && (tokio::select! {
                biased;
                changed = cancel_rx.changed() => {
                    if changed.is_ok() && *cancel_rx.borrow() {
                        return Err(TtsSegmentStreamError::client_disconnected(audio_started));
                    }
                    return Err(TtsSegmentStreamError::client_disconnected(audio_started));
                }
                result = out_tx.send(VoiceWsOut::Audio(chunk.to_vec())) => result,
            })
            .is_err()
        {
            return Err(TtsSegmentStreamError::client_disconnected(audio_started));
        }
        if !chunk.is_empty() {
            if first_audio_ms.is_none() {
                first_audio_ms = Some(started_at.elapsed().as_millis() as u64);
            }
            audio_chunks += 1;
            audio_bytes += chunk.len();
            audio_started = true;
        }
    }

    tracing::info!(
        conversation_id,
        voice_session_id,
        segment_index,
        is_first_segment = segment_index == 0,
        segment_chars = segment.chars().count(),
        queue_wait_ms = queue_wait.as_millis() as u64,
        first_audio_ms,
        duration_ms = started_at.elapsed().as_millis() as u64,
        audio_chunks,
        audio_bytes,
        "server-side voice TTS segment completed"
    );

    Ok(())
}

async fn acquire_voice_tts_permit<'a>(
    state: &'a AppState,
    cancel_rx: &mut watch::Receiver<bool>,
    conversation_id: Option<&str>,
    voice_session_id: u64,
    segment_index: usize,
) -> Result<tokio::sync::SemaphorePermit<'a>, VoiceTtsQueueWaitError> {
    let is_first_segment = segment_index == 0;
    let queue_timeout = server_tts_queue_timeout(is_first_segment);
    let acquire = state.voice_tts_semaphore.acquire();
    tokio::pin!(acquire);

    tokio::select! {
        biased;
        changed = cancel_rx.changed() => {
            if changed.is_ok() && *cancel_rx.borrow() {
                return Err(VoiceTtsQueueWaitError::Cancelled);
            }
            Err(VoiceTtsQueueWaitError::Cancelled)
        }
        result = &mut acquire => {
            result.map_err(|_| VoiceTtsQueueWaitError::Closed)
        }
        _ = tokio::time::sleep(queue_timeout) => {
            tracing::warn!(
                conversation_id,
                voice_session_id,
                segment_index,
                is_first_segment,
                queue_timeout_ms = queue_timeout.as_millis() as u64,
                "server-side voice TTS queue wait timed out"
            );
            Err(VoiceTtsQueueWaitError::TimedOut)
        }
    }
}

fn tts_log_preview(segment: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 48;
    let cleaned = clean_tts_segment(segment);
    let mut preview = cleaned.chars().take(MAX_PREVIEW_CHARS).collect::<String>();
    if cleaned.chars().count() > MAX_PREVIEW_CHARS {
        preview.push_str("...");
    }
    preview
}

fn tts_log_preview_for_env(segment: &str) -> String {
    if read_env_bool("VOICE_TTS_LOG_SEGMENT_PREVIEW").unwrap_or(false) {
        tts_log_preview(segment)
    } else {
        String::new()
    }
}

fn status_event(status: &'static str) -> Event {
    Event::default()
        .event("status")
        .data(json!({ "status": status }).to_string())
}

async fn list_majors(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").map(|value| value.trim().to_owned());
    match state.db.list_major_catalog().await {
        Ok(majors) => {
            let filtered = majors
                .into_iter()
                .filter(|major| {
                    query.as_ref().is_none_or(|query| {
                        major.name.contains(query) || major.slug.contains(query)
                    })
                })
                .map(|major| {
                    json!({
                        "id": major.slug,
                        "slug": major.slug,
                        "code": major.code.unwrap_or_default(),
                        "name": major.name,
                        "degreeLevel": null,
                        "durationYears": null,
                        "tuitionFee": null,
                        "isNormalMajor": major.is_normal_major,
                        "hasMaster": false,
                        "hasDoctor": false,
                        "university": { "code": "HRBNU", "name": "哈尔滨师范大学" },
                        "latestScore": null,
                        "tags": []
                    })
                })
                .collect::<Vec<_>>();
            (StatusCode::OK, Json(ok(json!(filtered)))).into_response()
        }
        Err(error) => {
            tracing::error!(error = %error, "failed to list majors");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("MAJORS_ERROR", "无法读取专业目录。")),
            )
                .into_response()
        }
    }
}

async fn get_major(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    match state.db.list_major_catalog().await {
        Ok(majors) => {
            let Some(major) = majors.into_iter().find(|major| major.slug == slug) else {
                return (
                    StatusCode::NOT_FOUND,
                    Json(fail("NOT_FOUND", format!("Major {slug} was not found"))),
                )
                    .into_response();
            };
            (
                StatusCode::OK,
                Json(ok(json!({
                    "id": major.slug,
                    "slug": major.slug,
                    "code": major.code.unwrap_or_default(),
                    "name": major.name,
                    "degreeLevel": null,
                    "durationYears": null,
                    "tuitionFee": null,
                    "isNormalMajor": major.is_normal_major,
                    "hasMaster": false,
                    "hasDoctor": false,
                    "introduction": null,
                    "employmentSummary": null,
                    "postgraduateSummary": null,
                    "university": { "code": "HRBNU", "name": "哈尔滨师范大学" },
                    "scoreTrend": [],
                    "planTrend": []
                }))),
            )
                .into_response()
        }
        Err(error) => {
            tracing::error!(error = %error, "failed to load major");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("MAJOR_ERROR", "无法读取专业详情。")),
            )
                .into_response()
        }
    }
}

async fn admission_scores(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let province = params.get("province").cloned().unwrap_or_default();
    let major_slug = params
        .get("majorSlug")
        .or_else(|| params.get("majorId"))
        .cloned()
        .unwrap_or_default();
    if province.is_empty() || major_slug.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(fail("BAD_REQUEST", "province and majorSlug are required")),
        )
            .into_response();
    }
    let year = params
        .get("year")
        .and_then(|value| value.parse::<i32>().ok());
    let subject_type = params.get("subjectType").map(String::as_str);
    match state
        .db
        .query_admission_scores(&province, &major_slug, subject_type, year)
        .await
    {
        Ok(records) => (StatusCode::OK, Json(ok(records))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to query admission scores");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("SCORES_ERROR", "无法读取录取分数。")),
            )
                .into_response()
        }
    }
}

async fn admission_plans_by_major() -> impl IntoResponse {
    (StatusCode::OK, Json(ok(json!([])))).into_response()
}

async fn knowledge_faq(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").cloned().unwrap_or_default();
    match state.db.search_faq(&query, 50).await {
        Ok(faq) => (StatusCode::OK, Json(ok(faq))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to search faq");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("FAQ_ERROR", "无法读取 FAQ。")),
            )
                .into_response()
        }
    }
}

async fn knowledge_policies(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").cloned().unwrap_or_default();
    let filters = db::KnowledgeSearchFilters {
        category: params.get("category").cloned(),
        year: params
            .get("year")
            .and_then(|value| value.parse::<i32>().ok()),
        document_kind: None,
    };
    match state.db.search_policies(&query, &filters, 50).await {
        Ok(policies) => (StatusCode::OK, Json(ok(policies))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to search policies");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("POLICY_ERROR", "无法读取政策资料。")),
            )
                .into_response()
        }
    }
}

async fn admin_dashboard(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    match state.db.admin_dashboard_snapshot().await {
        Ok(snapshot) => (StatusCode::OK, Json(ok(snapshot))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to load admin dashboard");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_DASHBOARD_ERROR", "无法读取后台看板数据。")),
            )
                .into_response()
        }
    }
}

async fn admin_insights(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    match state.db.admin_insights_snapshot().await {
        Ok(snapshot) => (StatusCode::OK, Json(ok(snapshot))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to load admin insights");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_INSIGHTS_ERROR", "无法读取热点分析数据。")),
            )
                .into_response()
        }
    }
}

async fn admin_special(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    match state.db.admin_special_snapshot().await {
        Ok(snapshot) => (StatusCode::OK, Json(ok(snapshot))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to load admin special analytics");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_SPECIAL_ERROR", "无法读取专项招生看板数据。")),
            )
                .into_response()
        }
    }
}

async fn admin_admissions_analytics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    match state.db.admin_admissions_analytics_snapshot().await {
        Ok(snapshot) => (StatusCode::OK, Json(ok(snapshot))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to load admin admissions analytics");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_ADMISSIONS_ERROR", "无法读取录取统计数据。")),
            )
                .into_response()
        }
    }
}

async fn admin_knowledge_coverage(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    match state.db.admin_knowledge_coverage_snapshot().await {
        Ok(snapshot) => (StatusCode::OK, Json(ok(snapshot))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to load admin knowledge coverage");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_KNOWLEDGE_COVERAGE_ERROR", "无法读取知识库覆盖数据。")),
            )
                .into_response()
        }
    }
}

async fn admin_big_screen(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    match state.db.admin_big_screen_snapshot().await {
        Ok(snapshot) => (StatusCode::OK, Json(ok(snapshot))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to load admin big screen");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_BIG_SCREEN_ERROR", "无法读取全国咨询态势数据。")),
            )
                .into_response()
        }
    }
}

async fn admin_conversations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    let query = params.get("q").cloned().unwrap_or_default();
    let (page, page_size) = pagination_from_params(&params);
    match state
        .db
        .admin_list_conversations(&query, page, page_size)
        .await
    {
        Ok(list) => (StatusCode::OK, Json(ok(list))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to list admin conversations");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_CONVERSATIONS_ERROR", "无法读取对话审计列表。")),
            )
                .into_response()
        }
    }
}

async fn admin_conversation_detail(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    if !is_valid_conversation_id(&conversation_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(fail("BAD_REQUEST", "Invalid conversation id")),
        )
            .into_response();
    }
    match state
        .db
        .admin_get_conversation_detail(&conversation_id)
        .await
    {
        Ok(Some(detail)) => (StatusCode::OK, Json(ok(detail))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(fail("NOT_FOUND", "Conversation not found")),
        )
            .into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to load admin conversation detail");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_CONVERSATION_ERROR", "无法读取对话详情。")),
            )
                .into_response()
        }
    }
}

async fn admin_knowledge_faqs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    let query = params.get("q").cloned().unwrap_or_default();
    let (page, page_size) = pagination_from_params(&params);
    match state.db.admin_list_faqs(&query, page, page_size).await {
        Ok(list) => (StatusCode::OK, Json(ok(list))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to list admin faqs");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_FAQ_ERROR", "无法读取后台 FAQ。")),
            )
                .into_response()
        }
    }
}

async fn admin_create_faq(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<AdminFaqWriteRequest>,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    let Some(question) = payload.question.as_deref().map(str::trim).filter(|value| !value.is_empty()) else {
        return (StatusCode::BAD_REQUEST, Json(fail("BAD_REQUEST", "FAQ 问题不能为空。"))).into_response();
    };
    let Some(answer) = payload.answer.as_deref().map(str::trim).filter(|value| !value.is_empty()) else {
        return (StatusCode::BAD_REQUEST, Json(fail("BAD_REQUEST", "FAQ 答案不能为空。"))).into_response();
    };
    if question.chars().count() > 300 || answer.chars().count() > 3000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(fail("BAD_REQUEST", "FAQ 问题或答案过长。")),
        )
            .into_response();
    }
    let category = payload
        .category
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("招生咨询");
    let source_label = payload
        .source_label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("管理后台录入");
    match state
        .db
        .admin_create_faq(
            question,
            answer,
            category,
            payload.tags.unwrap_or_default(),
            payload.status.as_deref().unwrap_or("draft"),
            source_label,
        )
        .await
    {
        Ok(item) => (StatusCode::OK, Json(ok(item))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to create admin faq");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_FAQ_CREATE_ERROR", "无法创建 FAQ。")),
            )
                .into_response()
        }
    }
}

async fn admin_update_faq(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(faq_id): Path<String>,
    Json(payload): Json<AdminFaqWriteRequest>,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    if faq_id.trim().is_empty() || faq_id.len() > 160 {
        return (
            StatusCode::BAD_REQUEST,
            Json(fail("BAD_REQUEST", "Invalid FAQ id")),
        )
            .into_response();
    }
    if payload
        .question
        .as_deref()
        .map(|value| value.chars().count() > 300)
        .unwrap_or(false)
        || payload
            .answer
            .as_deref()
            .map(|value| value.chars().count() > 3000)
            .unwrap_or(false)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(fail("BAD_REQUEST", "FAQ 问题或答案过长。")),
        )
            .into_response();
    }
    match state
        .db
        .admin_update_faq(
            &faq_id,
            payload.question.as_deref(),
            payload.answer.as_deref(),
            payload.category.as_deref(),
            payload.tags,
            payload.status.as_deref(),
            payload.source_label.as_deref(),
        )
        .await
    {
        Ok(Some(item)) => (StatusCode::OK, Json(ok(item))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(fail("NOT_FOUND", "FAQ not found")),
        )
            .into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to update admin faq");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_FAQ_UPDATE_ERROR", "无法更新 FAQ。")),
            )
                .into_response()
        }
    }
}

async fn admin_knowledge_chunks(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    let query = params.get("q").cloned().unwrap_or_default();
    let (page, page_size) = pagination_from_params(&params);
    match state
        .db
        .admin_list_knowledge_chunks(&query, page, page_size)
        .await
    {
        Ok(list) => (StatusCode::OK, Json(ok(list))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to list admin knowledge chunks");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_CHUNKS_ERROR", "无法读取后台知识片段。")),
            )
                .into_response()
        }
    }
}

async fn admin_create_feedback(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<AdminFeedbackRequest>,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    let comment = payload.comment.as_deref().map(str::trim).filter(|value| !value.is_empty());
    if comment.map(|value| value.chars().count()).unwrap_or(0) > 2000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(fail("BAD_REQUEST", "反馈备注过长，请控制在 2000 字以内。")),
        )
            .into_response();
    }
    match state
        .db
        .admin_create_feedback(
            payload.conversation_id.as_deref(),
            payload.message_id.as_deref(),
            &payload.feedback_type,
            comment,
            payload.handled_by.as_deref(),
            payload.status.as_deref().unwrap_or("open"),
        )
        .await
    {
        Ok(item) => (StatusCode::OK, Json(ok(item))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to create admin feedback");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_FEEDBACK_ERROR", "无法写入人工反馈。")),
            )
                .into_response()
        }
    }
}

async fn admin_tickets(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    let query = params.get("q").cloned().unwrap_or_default();
    let status = params.get("status").map(String::as_str);
    let (page, page_size) = pagination_from_params(&params);
    match state
        .db
        .admin_list_tickets(&query, status, page, page_size)
        .await
    {
        Ok(list) => (StatusCode::OK, Json(ok(list))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to list admin tickets");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_TICKETS_ERROR", "无法读取留言工单。")),
            )
                .into_response()
        }
    }
}

async fn admin_update_ticket(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(ticket_id): Path<String>,
    Json(payload): Json<AdminTicketUpdateRequest>,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    if ticket_id.trim().is_empty() || ticket_id.len() > 128 {
        return (
            StatusCode::BAD_REQUEST,
            Json(fail("BAD_REQUEST", "Invalid ticket id")),
        )
            .into_response();
    }
    match state
        .db
        .admin_update_ticket(
            &ticket_id,
            payload.status.as_deref(),
            payload.resolution.as_deref(),
            payload.handled_by.as_deref(),
        )
        .await
    {
        Ok(Some(item)) => (StatusCode::OK, Json(ok(item))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(fail("NOT_FOUND", "Ticket not found")),
        )
            .into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to update admin ticket");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_TICKET_UPDATE_ERROR", "无法更新留言工单。")),
            )
                .into_response()
        }
    }
}

async fn admin_settings(State(state): State<Arc<AppState>>, headers: HeaderMap) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    match state.db.admin_get_settings().await {
        Ok(settings) => (StatusCode::OK, Json(ok(settings))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to load admin settings");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_SETTINGS_ERROR", "无法读取系统配置。")),
            )
                .into_response()
        }
    }
}

async fn admin_update_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<AdminSettingsUpdateRequest>,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    if payload.welcome_message.trim().is_empty()
        || payload.fallback_message.trim().is_empty()
        || payload.welcome_message.chars().count() > 1000
        || payload.fallback_message.chars().count() > 1000
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(fail("BAD_REQUEST", "配置内容不能为空，且单项不超过 1000 字。")),
        )
            .into_response();
    }
    match state
        .db
        .admin_update_settings(
            &payload.welcome_message,
            &payload.fallback_message,
            payload.updated_by.as_deref(),
        )
        .await
    {
        Ok(settings) => (StatusCode::OK, Json(ok(settings))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to update admin settings");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_SETTINGS_UPDATE_ERROR", "无法保存系统配置。")),
            )
                .into_response()
        }
    }
}

async fn admin_audit_logs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    if let Err(response) = require_admin_token(&headers) {
        return response;
    }
    let (page, page_size) = pagination_from_params(&params);
    match state.db.admin_list_audit_logs(page, page_size).await {
        Ok(list) => (StatusCode::OK, Json(ok(list))).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "failed to list admin audit logs");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("ADMIN_AUDIT_LOG_ERROR", "无法读取操作日志。")),
            )
                .into_response()
        }
    }
}

fn pagination_from_params(params: &HashMap<String, String>) -> (i64, i64) {
    let page = params
        .get("page")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(1)
        .max(1);
    let page_size = params
        .get("pageSize")
        .or_else(|| params.get("page_size"))
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(20)
        .clamp(1, 100);
    (page, page_size)
}

fn require_admin_token(headers: &HeaderMap) -> Result<(), Response> {
    let Ok(expected) = std::env::var("ADMIN_API_TOKEN") else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(fail(
                "ADMIN_TOKEN_NOT_CONFIGURED",
                "后台管理接口尚未配置 ADMIN_API_TOKEN。",
            )),
        )
            .into_response());
    };
    let expected = expected.trim();
    if expected.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(fail(
                "ADMIN_TOKEN_NOT_CONFIGURED",
                "后台管理接口尚未配置 ADMIN_API_TOKEN。",
            )),
        )
            .into_response());
    }

    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);
    let header_token = headers
        .get("x-admin-token")
        .and_then(|value| value.to_str().ok())
        .map(str::trim);

    if bearer == Some(expected) || header_token == Some(expected) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(fail("UNAUTHORIZED", "后台管理接口未授权。")),
        )
            .into_response())
    }
}

async fn tts_token(State(state): State<Arc<AppState>>, headers: HeaderMap) -> impl IntoResponse {
    if !tts_auth_allowed(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(fail("UNAUTHORIZED", "TTS token access is not authorized")),
        )
            .into_response();
    }
    let rate_key = client_rate_key(&headers);
    if !state.tts_limiter.allow(&rate_key).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(fail(
                "TTS_RATE_LIMITED",
                "语音服务请求过于频繁，请稍后再试。",
            )),
        )
            .into_response();
    }

    let api_key = match std::env::var("DASHSCOPE_API_KEY") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail(
                    "TTS_CONFIG_ERROR",
                    "DASHSCOPE_API_KEY is not configured",
                )),
            )
                .into_response();
        }
    };

    let response = match state
        .tts_http
        .post("https://dashscope.aliyuncs.com/api/v1/tokens")
        .bearer_auth(api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::error!(error = %error, "failed to request DashScope TTS token");
            return (
                StatusCode::BAD_GATEWAY,
                Json(fail(
                    "TTS_TOKEN_ERROR",
                    "Failed to fetch temporary token from DashScope",
                )),
            )
                .into_response();
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body_len = response
            .text()
            .await
            .map(|body| body.len())
            .unwrap_or_default();
        tracing::error!(%status, body_len, "DashScope TTS token API returned an error");
        return (
            StatusCode::BAD_GATEWAY,
            Json(fail(
                "TTS_TOKEN_ERROR",
                "Failed to fetch temporary token from DashScope",
            )),
        )
            .into_response();
    }

    let payload = match response.json::<serde_json::Value>().await {
        Ok(payload) => payload,
        Err(error) => {
            tracing::error!(error = %error, "failed to parse DashScope TTS token response");
            return (
                StatusCode::BAD_GATEWAY,
                Json(fail(
                    "TTS_TOKEN_ERROR",
                    "Failed to parse temporary token from DashScope",
                )),
            )
                .into_response();
        }
    };

    let token = payload
        .get("token")
        .and_then(|value| value.as_str())
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| data.get("token"))
                .and_then(|value| value.as_str())
        });

    match token {
        Some(token) if !token.trim().is_empty() => {
            (StatusCode::OK, Json(ok(json!({ "token": token })))).into_response()
        }
        _ => {
            tracing::error!("DashScope returned empty TTS token");
            (
                StatusCode::BAD_GATEWAY,
                Json(fail("TTS_TOKEN_ERROR", "DashScope returned empty token")),
            )
                .into_response()
        }
    }
}

async fn tts_speech(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<TtsSpeechRequest>,
) -> Response {
    if !tts_auth_allowed(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(fail("UNAUTHORIZED", "TTS speech access is not authorized")),
        )
            .into_response();
    }
    let rate_key = client_rate_key(&headers);
    if !state.tts_limiter.allow(&rate_key).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(fail(
                "TTS_RATE_LIMITED",
                "语音服务请求过于频繁，请稍后再试。",
            )),
        )
            .into_response();
    }

    let input = payload.input.trim();
    if input.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(fail("TTS_INPUT_EMPTY", "语音合成文本不能为空。")),
        )
            .into_response();
    }
    let max_chars = read_env_usize("TTS_SPEECH_MAX_CHARS", 1600);
    if input.chars().count() > max_chars {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(fail(
                "TTS_INPUT_TOO_LONG",
                "语音合成文本过长，请缩短后重试。",
            )),
        )
            .into_response();
    }

    let endpoint = std::env::var("LOCAL_TTS_SPEECH_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:50000/v1/audio/speech".to_owned());
    let model = payload
        .model
        .or_else(|| std::env::var("LOCAL_TTS_MODEL").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "cosyvoice3".to_owned());
    let voice = payload
        .voice
        .or_else(|| std::env::var("LOCAL_TTS_VOICE").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "default".to_owned());

    let response = match state
        .tts_http
        .post(endpoint)
        .json(&json!({
            "model": model,
            "voice": voice,
            "input": input,
        }))
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::error!(error = %error, "failed to request local TTS speech");
            return (
                StatusCode::BAD_GATEWAY,
                Json(fail("TTS_SPEECH_ERROR", "语音服务暂时不可用，请稍后再试。")),
            )
                .into_response();
        }
    };

    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("audio/wav")
        .to_owned();

    if !status.is_success() {
        let body_len = response
            .text()
            .await
            .map(|body| body.len())
            .unwrap_or_default();
        tracing::error!(%status, body_len, "local TTS speech API returned an error");
        return (
            StatusCode::BAD_GATEWAY,
            Json(fail("TTS_SPEECH_ERROR", "语音服务暂时不可用，请稍后再试。")),
        )
            .into_response();
    }

    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::error!(error = %error, "failed to read local TTS speech body");
            return (
                StatusCode::BAD_GATEWAY,
                Json(fail("TTS_SPEECH_ERROR", "语音服务返回异常，请稍后再试。")),
            )
                .into_response();
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bytes))
        .unwrap_or_else(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("TTS_SPEECH_ERROR", "语音响应生成失败。")),
            )
                .into_response()
        })
}

async fn tts_stream(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<TtsSpeechRequest>,
) -> Response {
    if !tts_auth_allowed(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(fail("UNAUTHORIZED", "TTS stream access is not authorized")),
        )
            .into_response();
    }
    let rate_key = client_rate_key(&headers);
    if !state.tts_limiter.allow(&rate_key).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(fail(
                "TTS_RATE_LIMITED",
                "语音服务请求过于频繁，请稍后再试。",
            )),
        )
            .into_response();
    }

    let input = payload.input.trim();
    if input.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(fail("TTS_INPUT_EMPTY", "语音合成文本不能为空。")),
        )
            .into_response();
    }
    let max_chars = read_env_usize("TTS_SPEECH_MAX_CHARS", 1600);
    if input.chars().count() > max_chars {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(fail(
                "TTS_INPUT_TOO_LONG",
                "语音合成文本过长，请缩短后重试。",
            )),
        )
            .into_response();
    }

    let endpoint = std::env::var("LOCAL_TTS_STREAM_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:50000/v1/audio/stream".to_owned());
    let model = payload
        .model
        .or_else(|| std::env::var("LOCAL_TTS_MODEL").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "cosyvoice3".to_owned());
    let voice = payload
        .voice
        .or_else(|| std::env::var("LOCAL_TTS_VOICE").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "default".to_owned());

    let response = match state
        .tts_http
        .post(endpoint)
        .json(&json!({
            "model": model,
            "voice": voice,
            "input": input,
        }))
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::error!(error = %error, "failed to request local streaming TTS");
            return (
                StatusCode::BAD_GATEWAY,
                Json(fail("TTS_STREAM_ERROR", "语音服务暂时不可用，请稍后再试。")),
            )
                .into_response();
        }
    };

    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let sample_rate = response
        .headers()
        .get("x-audio-sample-rate")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("24000")
        .to_owned();

    if !status.is_success() {
        let body_len = response
            .text()
            .await
            .map(|body| body.len())
            .unwrap_or_default();
        tracing::error!(%status, body_len, "local streaming TTS API returned an error");
        return (
            StatusCode::BAD_GATEWAY,
            Json(fail("TTS_STREAM_ERROR", "语音服务暂时不可用，请稍后再试。")),
        )
            .into_response();
    }

    let stream = response.bytes_stream().map(|result| {
        result.map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("streaming TTS chunk failed: {error}"),
            )
        })
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-audio-format", "pcm_s16le")
        .header("x-audio-sample-rate", sample_rate)
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(fail("TTS_STREAM_ERROR", "语音流响应生成失败。")),
            )
                .into_response()
        })
}

impl ServerTtsSegmenterConfig {
    fn from_env() -> Self {
        Self {
            first_min_chars: read_env_usize("SERVER_TTS_FIRST_MIN_SEGMENT_CHARS", 8),
            first_max_chars: read_env_usize("SERVER_TTS_FIRST_MAX_SEGMENT_CHARS", 30),
            min_chars: read_env_usize("SERVER_TTS_MIN_SEGMENT_CHARS", 22),
            max_chars: read_env_usize("SERVER_TTS_MAX_SEGMENT_CHARS", 56),
            flush_after: Duration::from_millis(read_env_u64("SERVER_TTS_FLUSH_AFTER_MS", 420)),
        }
    }
}

impl ServerTtsSegmenter {
    fn new(config: ServerTtsSegmenterConfig) -> Self {
        Self {
            buffer: String::new(),
            emitted_count: 0,
            config,
        }
    }

    fn is_empty(&self) -> bool {
        clean_tts_segment(&self.buffer).is_empty()
    }

    fn push(&mut self, delta: &str) -> Vec<String> {
        let normalized = normalize_tts_delta(delta);
        if !normalized.is_empty() {
            if needs_space_between(&self.buffer, &normalized) {
                self.buffer.push(' ');
            }
            self.buffer.push_str(&normalized);
        }
        self.drain_ready(false)
    }

    fn flush_latency(&mut self) -> Option<String> {
        let min_chars = if self.emitted_count == 0 {
            self.config.first_min_chars
        } else {
            self.config.min_chars
        };
        if clean_tts_segment(&self.buffer).chars().count() < min_chars {
            return None;
        }
        self.take_prefix(best_latency_split_index(
            &self.buffer,
            self.effective_max_chars(),
        ))
    }

    fn finish(&mut self) -> Vec<String> {
        self.drain_ready(true)
    }

    fn drain_ready(&mut self, flush: bool) -> Vec<String> {
        let mut segments = Vec::new();
        loop {
            self.trim_start();
            if self.buffer.is_empty() {
                break;
            }

            let min_chars = if self.emitted_count == 0 {
                self.config.first_min_chars
            } else {
                self.config.min_chars
            };

            let split_index = find_sentence_split_index(&self.buffer, min_chars)
                .or_else(|| find_max_split_index(&self.buffer, self.effective_max_chars()))
                .or_else(|| if flush { Some(self.buffer.len()) } else { None });

            let Some(split_index) = split_index else {
                break;
            };
            let Some(segment) = self.take_prefix(split_index) else {
                break;
            };
            segments.push(segment);
        }
        segments
    }

    fn take_prefix(&mut self, split_index: usize) -> Option<String> {
        let split_index = split_index.min(self.buffer.len());
        let remaining = self.buffer.split_off(split_index);
        let segment = clean_tts_segment(&self.buffer);
        self.buffer = remaining;
        if segment.is_empty() {
            return None;
        }
        self.emitted_count += 1;
        Some(segment)
    }

    fn trim_start(&mut self) {
        let trimmed = self.buffer.trim_start();
        if trimmed.len() != self.buffer.len() {
            self.buffer = trimmed.to_owned();
        }
    }

    fn effective_max_chars(&self) -> usize {
        if self.emitted_count == 0 {
            self.config.first_max_chars
        } else {
            self.config.max_chars
        }
    }
}

fn server_tts_segmenter_config() -> ServerTtsSegmenterConfig {
    ServerTtsSegmenterConfig::from_env()
}

fn normalize_tts_delta(delta: &str) -> String {
    let mut out = String::new();
    let mut previous_space = false;
    for ch in delta.chars() {
        if matches!(
            ch,
            '`' | '*' | '_' | '#' | '>' | '[' | ']' | '(' | ')' | '{' | '}'
        ) {
            continue;
        }
        if ch.is_whitespace() {
            if !previous_space && !out.is_empty() {
                out.push(' ');
                previous_space = true;
            }
            continue;
        }
        out.push(ch);
        previous_space = false;
    }
    out
}

fn clean_tts_segment(segment: &str) -> String {
    let normalized = normalize_tts_delta(segment);
    normalized
        .trim_matches(|ch: char| matches!(ch, '-' | '|' | ':' | '：' | ',' | '，' | '、'))
        .trim()
        .to_owned()
}

fn record_spoken_tts_delta(buffer: &Arc<std::sync::Mutex<String>>, delta: &str) {
    let cleaned = normalize_tts_delta(delta).trim().to_owned();
    if cleaned.is_empty() {
        return;
    }

    let Ok(mut spoken) = buffer.lock() else {
        return;
    };
    if needs_space_between(&spoken, &cleaned) {
        spoken.push(' ');
    }
    spoken.push_str(&cleaned);
}

fn unspoken_tts_suffix(
    spoken_tts_text: &Arc<std::sync::Mutex<String>>,
    final_reply: &str,
) -> Option<String> {
    let final_cleaned = clean_tts_segment(final_reply);
    if final_cleaned.is_empty() {
        return None;
    }

    let spoken = spoken_tts_text
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    let spoken_cleaned = clean_tts_segment(&spoken);
    if spoken_cleaned.is_empty() {
        return Some(final_cleaned);
    }
    if final_cleaned == spoken_cleaned {
        return None;
    }
    if effectively_same_tts_text(&spoken_cleaned, &final_cleaned) {
        return None;
    }
    if let Some(suffix) = final_cleaned.strip_prefix(&spoken_cleaned) {
        let suffix = clean_tts_segment(suffix);
        return (!suffix.is_empty()).then_some(suffix);
    }

    let Some(overlap_len) = longest_suffix_prefix_overlap(&spoken_cleaned, &final_cleaned, 12)
    else {
        tracing::warn!(
            spoken_chars = spoken_cleaned.chars().count(),
            final_chars = final_cleaned.chars().count(),
            "server-side voice final reply did not align with streamed TTS text; skipped replay to avoid duplicate speech"
        );
        return None;
    };

    let suffix = clean_tts_segment(&final_cleaned[overlap_len..]);
    (!suffix.is_empty()).then_some(suffix)
}

fn longest_suffix_prefix_overlap(left: &str, right: &str, min_chars: usize) -> Option<usize> {
    let mut best = None;
    for (byte_index, _) in left.char_indices() {
        let suffix = &left[byte_index..];
        if suffix.chars().count() < min_chars {
            break;
        }
        if right.starts_with(suffix) {
            best = Some(suffix.len());
            break;
        }
    }
    best
}

fn effectively_same_tts_text(left: &str, right: &str) -> bool {
    let left_compact = compact_tts_match_text(left);
    let right_compact = compact_tts_match_text(right);
    if !left_compact.is_empty() && left_compact == right_compact {
        return true;
    }

    let left_len = left.chars().count();
    let right_len = right.chars().count();
    let max_len = left_len.max(right_len);
    if max_len == 0 || left_len.abs_diff(right_len) > 4 {
        return false;
    }

    common_prefix_chars(left, right) + 4 >= max_len
}

fn common_prefix_chars(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .count()
}

fn compact_tts_match_text(text: &str) -> String {
    text.chars()
        .filter(|ch| {
            !ch.is_whitespace()
                && !matches!(
                    ch,
                    '。' | '，'
                        | '、'
                        | '；'
                        | '：'
                        | '！'
                        | '？'
                        | '.'
                        | ','
                        | ';'
                        | ':'
                        | '!'
                        | '?'
                        | '-'
                        | '|'
                        | '"'
                        | '\''
                        | '“'
                        | '”'
                        | '‘'
                        | '’'
                        | '（'
                        | '）'
                        | '('
                        | ')'
                )
        })
        .collect()
}

fn needs_space_between(current: &str, next: &str) -> bool {
    let Some(last) = current.chars().last() else {
        return false;
    };
    let Some(first) = next.chars().next() else {
        return false;
    };
    last.is_ascii_alphanumeric() && first.is_ascii_alphanumeric()
}

fn find_sentence_split_index(text: &str, min_chars: usize) -> Option<usize> {
    let mut count = 0usize;
    for (index, ch) in text.char_indices() {
        count += 1;
        if count >= min_chars && matches!(ch, '。' | '！' | '？' | '!' | '?' | '；' | ';' | '\n')
        {
            return Some(index + ch.len_utf8());
        }
    }
    None
}

fn find_max_split_index(text: &str, max_chars: usize) -> Option<usize> {
    let mut count = 0usize;
    let mut fallback = None;
    let mut natural = None;
    for (index, ch) in text.char_indices() {
        count += 1;
        if count >= 12 && matches!(ch, '，' | ',' | '、' | '：' | ':' | ' ') {
            natural = Some(index + ch.len_utf8());
        }
        if count == max_chars {
            fallback = Some(index + ch.len_utf8());
        }
        if count > max_chars {
            return natural.or(fallback);
        }
    }
    None
}

fn best_latency_split_index(text: &str, max_chars: usize) -> usize {
    find_sentence_split_index(text, 8)
        .or_else(|| find_max_split_index(text, max_chars))
        .unwrap_or(text.len())
}

fn local_tts_stream_url() -> String {
    std::env::var("LOCAL_TTS_STREAM_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:50000/v1/audio/stream".to_owned())
}

fn local_tts_model() -> String {
    std::env::var("LOCAL_TTS_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "cosyvoice3".to_owned())
}

fn local_tts_voice() -> String {
    std::env::var("LOCAL_TTS_VOICE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "default".to_owned())
}

fn server_tts_segment_retries() -> usize {
    read_env_usize("SERVER_TTS_SEGMENT_RETRIES", 1)
}

fn server_tts_max_consecutive_failures() -> usize {
    read_env_usize("SERVER_TTS_MAX_CONSECUTIVE_FAILURES", 3).max(1)
}

fn server_tts_retry_delay() -> Duration {
    Duration::from_millis(read_env_u64("SERVER_TTS_RETRY_DELAY_MS", 160))
}

fn server_tts_max_concurrent_synth() -> usize {
    read_env_usize("SERVER_TTS_MAX_CONCURRENT_SYNTH", 2).max(1)
}

fn server_tts_queue_timeout(is_first_segment: bool) -> Duration {
    let legacy = read_env_u64("SERVER_TTS_QUEUE_TIMEOUT_MS", 20_000);
    let key = if is_first_segment {
        "SERVER_TTS_FIRST_QUEUE_TIMEOUT_MS"
    } else {
        "SERVER_TTS_CONTINUATION_QUEUE_TIMEOUT_MS"
    };
    let fallback = if is_first_segment { legacy } else { 120_000 };
    Duration::from_millis(read_env_u64(key, fallback))
}

impl TtsTokenLimiter {
    fn new(limit_per_minute: u32) -> Self {
        Self {
            limit_per_minute: limit_per_minute.max(1),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    async fn allow(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().await;
        if buckets.len() > 4096 {
            buckets.retain(|_, bucket| {
                now.duration_since(bucket.window_started_at) < Duration::from_secs(60)
            });
        }

        let bucket = buckets
            .entry(key.to_owned())
            .or_insert_with(|| TtsRateBucket {
                window_started_at: now,
                count: 0,
            });
        if now.duration_since(bucket.window_started_at) >= Duration::from_secs(60) {
            bucket.window_started_at = now;
            bucket.count = 0;
        }
        if bucket.count >= self.limit_per_minute {
            return false;
        }
        bucket.count += 1;
        true
    }
}

fn cors_layer() -> CorsLayer {
    let origins = read_allowed_origins();
    let layer = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    if origins.is_empty() && !is_production() {
        layer.allow_origin(Any)
    } else {
        layer.allow_origin(AllowOrigin::list(origins))
    }
}

fn read_allowed_origins() -> Vec<HeaderValue> {
    std::env::var("CORS_ALLOWED_ORIGINS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|origin| {
            let origin = origin.trim();
            if origin.is_empty() {
                None
            } else {
                origin.parse::<HeaderValue>().ok()
            }
        })
        .collect()
}

fn build_tts_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(read_env_u64(
            "TTS_CONNECT_TIMEOUT_SECS",
            5,
        )))
        .timeout(Duration::from_secs(read_env_u64(
            "TTS_REQUEST_TIMEOUT_SECS",
            15,
        )))
        .pool_idle_timeout(Duration::from_secs(read_env_u64(
            "TTS_POOL_IDLE_TIMEOUT_SECS",
            30,
        )))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn tts_auth_allowed(headers: &HeaderMap) -> bool {
    let Some(expected) = std::env::var("TTS_TOKEN_AUTH_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return true;
    };
    let expected = expected.trim();

    let bearer = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);
    let header_token = headers
        .get("x-tts-auth-token")
        .and_then(|value| value.to_str().ok())
        .map(str::trim);

    bearer == Some(expected) || header_token == Some(expected)
}

fn client_rate_key(headers: &HeaderMap) -> String {
    for header in ["x-forwarded-for", "x-real-ip"] {
        if let Some(value) = headers.get(header).and_then(|value| value.to_str().ok()) {
            if let Some(first) = value
                .split(',')
                .next()
                .map(str::trim)
                .filter(|item| !item.is_empty())
            {
                return first.chars().take(80).collect();
            }
        }
    }
    "unknown-client".to_owned()
}

fn client_diagnostics_enabled() -> bool {
    if let Some(value) = read_env_bool("ENABLE_CLIENT_DIAGNOSTICS") {
        return value;
    }
    !is_production()
}

fn is_production() -> bool {
    ["APP_ENV", "RUST_ENV", "NODE_ENV"]
        .iter()
        .any(|key| std::env::var(key).is_ok_and(|value| value.eq_ignore_ascii_case("production")))
}

fn agent_timeout_duration() -> Duration {
    Duration::from_secs(read_env_u64("AGENT_TIMEOUT_SECS", 75))
}

fn read_env_bool(key: &str) -> Option<bool> {
    std::env::var(key).ok().and_then(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        }
    })
}

fn read_env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn read_env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn read_env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn is_valid_conversation_id(value: &str) -> bool {
    let len = value.chars().count();
    (8..=96).contains(&len)
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spoken_buffer(text: &str) -> Arc<std::sync::Mutex<String>> {
        Arc::new(std::sync::Mutex::new(text.to_owned()))
    }

    #[test]
    fn unspoken_tts_suffix_returns_full_reply_when_nothing_was_streamed() {
        let spoken = spoken_buffer("");

        assert_eq!(
            unspoken_tts_suffix(&spoken, "欢迎咨询哈尔滨师范大学招生信息。"),
            Some("欢迎咨询哈尔滨师范大学招生信息。".to_owned())
        );
    }

    #[test]
    fn unspoken_tts_suffix_returns_only_missing_tail() {
        let spoken = spoken_buffer("前半段已经播报。");

        assert_eq!(
            unspoken_tts_suffix(&spoken, "前半段已经播报。后半段需要补上。"),
            Some("后半段需要补上。".to_owned())
        );
    }

    #[test]
    fn unspoken_tts_suffix_ignores_markdown_noise_for_matching() {
        let spoken = spoken_buffer("这是第一句。");

        assert_eq!(
            unspoken_tts_suffix(&spoken, "**这是第一句。** 这是第二句。"),
            Some("这是第二句。".to_owned())
        );
    }

    #[test]
    fn unspoken_tts_suffix_avoids_duplicate_replay_when_already_complete() {
        let spoken = spoken_buffer("已经完整播报。");

        assert_eq!(unspoken_tts_suffix(&spoken, "已经完整播报。"), None);
    }

    #[test]
    fn unspoken_tts_suffix_treats_tiny_tail_differences_as_complete() {
        let spoken = spoken_buffer("这段内容基本已经播报完成");

        assert_eq!(
            unspoken_tts_suffix(&spoken, "这段内容基本已经播报完成。"),
            None
        );
    }

    #[test]
    fn unspoken_tts_suffix_ignores_internal_punctuation_differences() {
        let spoken = spoken_buffer("你好我是招生助手可以帮你查分数线");

        assert_eq!(
            unspoken_tts_suffix(&spoken, "你好，我是招生助手，可以帮你查分数线。"),
            None
        );
    }
}
