use admissions_agent::AdmissionsAgent;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, Sse},
    },
    routing::{get, post},
};
use db::Database;
use domain::{ChatRequest, fail, ok, ok_with_meta};
use serde_json::json;
use std::collections::HashMap;
use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore, mpsc};
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
}

struct TtsRateBucket {
    window_started_at: Instant,
    count: u32,
}

struct TtsTokenLimiter {
    limit_per_minute: u32,
    buckets: Mutex<HashMap<String, TtsRateBucket>>,
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
        .route("/api/v1/tts/token", post(tts_token))
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
            let meta = if client_diagnostics_enabled() {
                reply
                    .diagnostics
                    .as_ref()
                    .map(|diagnostics| json!({ "diagnostics": diagnostics }))
                    .unwrap_or_else(|| json!({}))
            } else {
                json!({})
            };
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

                let meta = if client_diagnostics_enabled() {
                    reply
                        .diagnostics
                        .as_ref()
                        .map(|diagnostics| json!({ "diagnostics": diagnostics }))
                        .unwrap_or_else(|| json!({}))
                } else {
                    json!({})
                };
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
