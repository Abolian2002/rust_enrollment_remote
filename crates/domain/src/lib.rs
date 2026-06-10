use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatProfile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub province: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<ChatProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatCitation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    pub source_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMemory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub province_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub province_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub major_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub major_name: Option<String>,
    #[serde(default)]
    pub intended_majors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_intent: Option<ChatIntent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatIntent {
    Greeting,
    ProbabilityAssessment,
    ScoreQuery,
    KnowledgeAnswer,
    GeneralAnswer,
    FallbackReply,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MajorCandidate {
    pub slug: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default)]
    pub is_normal_major: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_score: Option<LatestScore>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LatestScore {
    pub year: i32,
    pub min_score: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionScoreRecord {
    pub year: i32,
    pub batch: String,
    pub subject_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admitted_count: Option<i32>,
    pub min_score: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_score: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_rank: Option<i32>,
    pub source_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProvinceAdmissionMajor {
    pub year: i32,
    pub major_name: String,
    pub subject_type: String,
    pub batch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admitted_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_score: Option<i32>,
    pub source_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MajorAdmissionProvince {
    pub year: i32,
    pub province_name: String,
    pub subject_type: String,
    pub batch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admitted_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_score: Option<i32>,
    pub source_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FaqEvidence {
    pub id: String,
    pub question: String,
    pub answer: String,
    pub category: String,
    pub source_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PolicyEvidence {
    pub id: String,
    pub title: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    pub source_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    pub content_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VectorChunkEvidence {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f64>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScoreSummary {
    pub total_records: usize,
    pub years: Vec<i32>,
    pub source_labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ChatStructuredResult {
    Greeting {
        message: String,
    },
    FollowUp {
        pending_intent: ChatIntent,
        missing_fields: Vec<String>,
        collected_profile: ResolvedMemory,
    },
    ScoreQuery {
        major_name: String,
        province: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        subject_type: Option<String>,
        records: Vec<AdmissionScoreRecord>,
        summary: ScoreSummary,
        #[serde(skip_serializing_if = "Option::is_none")]
        diagnostics: Option<Value>,
    },
    ProbabilityAssessment {
        assessment: Value,
    },
    KnowledgeAnswer {
        query: String,
        faq: Vec<FaqEvidence>,
        policies: Vec<PolicyEvidence>,
        #[serde(default)]
        vector_chunks: Vec<VectorChunkEvidence>,
    },
    ProvinceMajorList {
        province: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        subject_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        year: Option<i32>,
        majors: Vec<ProvinceAdmissionMajor>,
        source_mode: String,
        note: String,
    },
    MajorProvinceList {
        major_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        subject_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        year: Option<i32>,
        provinces: Vec<MajorAdmissionProvince>,
        source_mode: String,
        note: String,
    },
    MajorDisambiguation {
        query: String,
        pending_intent: ChatIntent,
        candidates: Vec<MajorCandidate>,
        missing_fields: Vec<String>,
        message: String,
    },
    EvidenceBundle {
        message: String,
        results: Vec<ChatStructuredResult>,
    },
    GeneralAnswer {
        answer: String,
        redirect_prompt: String,
        collected_profile: ResolvedMemory,
    },
    FallbackReply {
        message: String,
    },
}

impl ChatStructuredResult {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Greeting { .. } => "greeting",
            Self::FollowUp { .. } => "follow_up",
            Self::ScoreQuery { .. } => "score_query",
            Self::ProbabilityAssessment { .. } => "probability_assessment",
            Self::KnowledgeAnswer { .. } => "knowledge_answer",
            Self::ProvinceMajorList { .. } => "province_major_list",
            Self::MajorProvinceList { .. } => "major_province_list",
            Self::MajorDisambiguation { .. } => "major_disambiguation",
            Self::EvidenceBundle { .. } => "evidence_bundle",
            Self::GeneralAnswer { .. } => "general_answer",
            Self::FallbackReply { .. } => "fallback_reply",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceStep {
    pub step: usize,
    pub node: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatDiagnostics {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_intent: Option<ChatIntent>,
    pub total_duration_ms: u128,
    pub model_call_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_model: Option<String>,
    #[serde(default)]
    pub synthesis_used: bool,
    pub tool_call_count: usize,
    #[serde(default)]
    pub trace: Vec<AgentTraceStep>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression: Option<ContextCompressionDiagnostics>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContextCompressionDiagnostics {
    pub applied: bool,
    pub level: String,
    pub original_token_estimate: usize,
    pub compressed_token_estimate: usize,
    pub threshold_token_estimate: usize,
    pub recent_message_count: usize,
    pub summary_token_estimate: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatReply {
    pub conversation_id: String,
    pub reply: String,
    pub structured_result: ChatStructuredResult,
    pub citations: Vec<ChatCitation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<ChatDiagnostics>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_payload: Option<ChatStructuredResult>,
    #[serde(default)]
    pub citations: Vec<ChatCitation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationHistory {
    pub id: String,
    pub session_key: String,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdminStat {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdminChartDatum {
    pub name: String,
    pub value: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdminDashboardSnapshot {
    pub updated_at: String,
    pub stats: Vec<AdminStat>,
    pub trend_days: Vec<String>,
    pub trend_values: Vec<i64>,
    pub hourly_values: Vec<i64>,
    pub hot_questions: Vec<(String, String)>,
    pub category_stats: Vec<AdminChartDatum>,
    pub province_bars: Vec<(String, i64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdminConversationListItem {
    pub id: String,
    pub province: String,
    pub updated_at: String,
    pub message_count: i64,
    pub status: String,
    pub manual_intervention: bool,
    pub last_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdminConversationList {
    pub items: Vec<AdminConversationListItem>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdminConversationDetail {
    pub id: String,
    pub province: String,
    pub status: String,
    pub manual_intervention: bool,
    pub message_count: usize,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdminFaqItem {
    pub id: String,
    pub question: String,
    pub similar: String,
    pub answer: String,
    pub source: String,
    pub updated_at: String,
    pub status: String,
    pub hits: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdminFaqList {
    pub items: Vec<AdminFaqItem>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdminKnowledgeChunkItem {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub excerpt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub college: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub major_name: Option<String>,
    pub source_type: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdminKnowledgeChunkList {
    pub items: Vec<AdminKnowledgeChunkItem>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiEnvelope<T>
where
    T: Serialize,
{
    pub success: bool,
    pub data: Option<T>,
    pub meta: Value,
    pub error: Option<ApiErrorBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
}

pub fn ok<T: Serialize>(data: T) -> ApiEnvelope<T> {
    ApiEnvelope {
        success: true,
        data: Some(data),
        meta: Value::Object(Default::default()),
        error: None,
    }
}

pub fn ok_with_meta<T: Serialize>(data: T, meta: Value) -> ApiEnvelope<T> {
    ApiEnvelope {
        success: true,
        data: Some(data),
        meta,
        error: None,
    }
}

pub fn fail(code: impl Into<String>, message: impl Into<String>) -> ApiEnvelope<Value> {
    ApiEnvelope {
        success: false,
        data: None,
        meta: Value::Object(Default::default()),
        error: Some(ApiErrorBody {
            code: code.into(),
            message: message.into(),
        }),
    }
}
