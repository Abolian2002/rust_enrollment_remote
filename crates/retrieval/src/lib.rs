use anyhow::Result;
use db::{
    CollegeMajorRecord, Database, KnowledgeSearchFilters, MajorRecord, summarize_score_records,
};
use domain::{
    AdmissionScoreRecord, ChatCitation, ChatIntent, ChatStructuredResult, FaqEvidence,
    MajorCandidate, PolicyEvidence, VectorChunkEvidence,
};
use embeddings::EmbeddingClient;
use serde_json::json;
use std::cmp::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

const DEFAULT_FAQ_MIN_SIMILARITY: f64 = 0.66;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetrievalIntent {
    Greeting,
    ScoreQuery,
    ProbabilityAssessment,
    KnowledgeAnswer,
    GeneralAnswer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision {
    pub intent: RetrievalIntent,
    pub must_use_tools: bool,
    pub reason: String,
}

#[derive(Clone)]
pub struct RetrievalService {
    db: Database,
    embeddings: EmbeddingClient,
    major_catalog_cache: Arc<RwLock<MajorCatalogCache>>,
}

#[derive(Debug, Default)]
struct MajorCatalogCache {
    loaded_at: Option<Instant>,
    majors: Vec<MajorRecord>,
}

impl RetrievalService {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            embeddings: EmbeddingClient::from_env(),
            major_catalog_cache: Arc::new(RwLock::new(MajorCatalogCache::default())),
        }
    }

    pub fn db(&self) -> &Database {
        &self.db
    }

    pub async fn search_major_candidates(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MajorCandidate>> {
        let majors = self.major_catalog_snapshot().await?;
        let mut scored = majors
            .into_iter()
            .map(|major| {
                let score = score_major_candidate(query, &major);
                (major, score)
            })
            .filter(|(_, score)| *score >= 25)
            .collect::<Vec<_>>();

        scored.sort_by(|(left_major, left_score), (right_major, right_score)| {
            right_score
                .cmp(left_score)
                .then_with(|| left_major.name.cmp(&right_major.name))
        });

        let mut results = Vec::new();
        let mut root_counts: Vec<(String, usize)> = Vec::new();
        for (major, _) in scored {
            let root = root_major_name(&major.name);
            let count = root_counts
                .iter_mut()
                .find(|(item, _)| item == &root)
                .map(|(_, count)| {
                    *count += 1;
                    *count
                })
                .unwrap_or_else(|| {
                    root_counts.push((root, 1));
                    1
                });
            if count > 3 {
                continue;
            }

            results.push(MajorCandidate {
                slug: major.slug,
                name: major.name,
                code: major.code,
                is_normal_major: major.is_normal_major,
                latest_score: major.latest_score,
            });
            if results.len() >= limit {
                break;
            }
        }

        Ok(results)
    }

    async fn major_catalog_snapshot(&self) -> Result<Vec<MajorRecord>> {
        let ttl = major_catalog_cache_ttl();
        {
            let cache = self.major_catalog_cache.read().await;
            if cache
                .loaded_at
                .is_some_and(|loaded_at| loaded_at.elapsed() < ttl)
            {
                return Ok(cache.majors.clone());
            }
        }

        let mut cache = self.major_catalog_cache.write().await;
        if cache
            .loaded_at
            .is_some_and(|loaded_at| loaded_at.elapsed() < ttl)
        {
            return Ok(cache.majors.clone());
        }

        let majors = self.db.list_major_catalog_with_latest_scores().await?;
        cache.loaded_at = Some(Instant::now());
        cache.majors = majors.clone();
        Ok(majors)
    }

    pub async fn list_college_majors(&self, college_name: &str) -> Result<Vec<CollegeMajorRecord>> {
        self.db
            .list_college_training_plan_majors(college_name)
            .await
    }

    pub async fn query_scores(
        &self,
        province: &str,
        major_slug: &str,
        major_name: &str,
        subject_type: Option<&str>,
        year: Option<i32>,
    ) -> Result<ChatStructuredResult> {
        let records = self
            .db
            .query_admission_scores(province, major_slug, subject_type, year)
            .await?;
        let summary = summarize_score_records(&records);
        let resolved_subject_type = resolve_score_query_subject_type(subject_type, &records);
        let diagnostics =
            score_query_subject_diagnostics(subject_type, resolved_subject_type.as_deref());
        Ok(ChatStructuredResult::ScoreQuery {
            major_name: major_name.to_owned(),
            province: province.to_owned(),
            subject_type: resolved_subject_type,
            records,
            summary,
            diagnostics,
        })
    }

    pub async fn list_province_admission_majors(
        &self,
        province: &str,
        subject_type: Option<&str>,
        year: Option<i32>,
    ) -> Result<ChatStructuredResult> {
        let majors = self
            .db
            .list_province_admission_majors_from_scores(province, subject_type, year, 120)
            .await?;
        let latest_year = majors.first().map(|item| item.year);
        Ok(ChatStructuredResult::ProvinceMajorList {
            province: province.to_owned(),
            subject_type: subject_type.map(ToOwned::to_owned),
            year: year.or(latest_year),
            majors,
            source_mode: "admission_scores_latest_year".to_owned(),
            note: "当前 admission_plans 招生计划表为空；这里使用已导入录取统计中最新一年在该省有录取记录的专业作为参考，不等同于当年正式招生计划。".to_owned(),
        })
    }

    pub async fn list_major_admission_provinces(
        &self,
        major_slug: &str,
        major_name: &str,
        subject_type: Option<&str>,
        year: Option<i32>,
    ) -> Result<ChatStructuredResult> {
        let provinces = self
            .db
            .list_major_admission_provinces_from_scores(major_slug, subject_type, year, 120)
            .await?;
        let latest_year = provinces.first().map(|item| item.year);
        Ok(ChatStructuredResult::MajorProvinceList {
            major_name: major_name.to_owned(),
            subject_type: subject_type.map(ToOwned::to_owned),
            year: year.or(latest_year),
            provinces,
            source_mode: "admission_scores_latest_year".to_owned(),
            note: "当前 admission_plans 招生计划表为空；这里使用已导入录取统计中最新一年该专业有录取记录的省份作为参考，不等同于当年正式招生计划。".to_owned(),
        })
    }

    pub async fn retrieve_knowledge(&self, query: &str) -> Result<KnowledgeRetrievalResult> {
        self.retrieve_knowledge_with_focus(query, None).await
    }

    pub async fn retrieve_knowledge_for_major(
        &self,
        query: &str,
        major_focus: &str,
    ) -> Result<KnowledgeRetrievalResult> {
        self.retrieve_knowledge_with_focus(query, Some(major_focus))
            .await
    }

    async fn retrieve_knowledge_with_focus(
        &self,
        query: &str,
        forced_major_focus: Option<&str>,
    ) -> Result<KnowledgeRetrievalResult> {
        let filters = infer_knowledge_filters(query);
        let major_focus = if filters.document_kind.as_deref() == Some("training_plan") {
            if let Some(major_focus) = forced_major_focus
                .and_then(sanitize_major_focus_for_retrieval)
                .filter(|value| !value.trim().is_empty())
            {
                Some(major_focus)
            } else if let Some(major_focus) = extract_major_focus_from_query(query) {
                Some(major_focus)
            } else {
                self.search_major_candidates(query, 1)
                    .await
                    .ok()
                    .and_then(|candidates| candidates.into_iter().next())
                    .map(|candidate| candidate.name)
            }
        } else {
            None
        };
        let (mut faq, mut policies, mut chunks) = self
            .retrieve_knowledge_once(query, &filters, major_focus.as_deref())
            .await?;
        if faq.is_empty() && policies.is_empty() && chunks.is_empty() {
            for fallback_query in fallback_knowledge_queries(query) {
                let (fallback_faq, fallback_policies, fallback_chunks) = self
                    .retrieve_knowledge_once(&fallback_query, &filters, major_focus.as_deref())
                    .await?;
                merge_faq(&mut faq, fallback_faq);
                merge_policies(&mut policies, fallback_policies);
                merge_chunks(&mut chunks, fallback_chunks);
                if !faq.is_empty() || !policies.is_empty() || !chunks.is_empty() {
                    break;
                }
            }
        }
        if filters.document_kind.as_deref() == Some("training_plan") && chunks.is_empty() {
            for fallback_query in fallback_knowledge_queries(query) {
                let (_, _, fallback_chunks) = self
                    .retrieve_knowledge_once(&fallback_query, &filters, major_focus.as_deref())
                    .await?;
                merge_chunks(&mut chunks, fallback_chunks);
                if !chunks.is_empty() {
                    break;
                }
            }
        }
        let faq = grade_faq(faq);
        let mut graded_chunks = grade_chunks(
            chunks,
            query,
            major_focus.as_deref(),
            filters.document_kind.as_deref() == Some("training_plan"),
        );
        if graded_chunks.is_empty()
            && (filters.document_kind.as_deref() == Some("training_plan")
                || query_looks_like_training_plan(query))
        {
            for major_query in training_plan_major_fallback_queries(query, major_focus.as_deref()) {
                let fallback_chunks = self
                    .db
                    .search_knowledge_chunks_keyword(&major_query, &filters, 12)
                    .await
                    .unwrap_or_default();
                graded_chunks = grade_chunks(fallback_chunks, query, Some(&major_query), true);
                if graded_chunks.is_empty() {
                    let direct_chunks = self
                        .db
                        .search_training_plan_chunks_by_major(
                            &major_query,
                            knowledge_topic_keyword(query),
                            12,
                        )
                        .await
                        .unwrap_or_default();
                    graded_chunks = grade_chunks(direct_chunks, query, Some(&major_query), true);
                }
                if !graded_chunks.is_empty() {
                    break;
                }
            }
        }
        let citations = build_knowledge_citations(&faq, &policies, &graded_chunks);
        Ok(KnowledgeRetrievalResult {
            structured_result: ChatStructuredResult::KnowledgeAnswer {
                query: query.to_owned(),
                faq,
                policies,
                vector_chunks: graded_chunks,
            },
            citations,
        })
    }

    async fn retrieve_knowledge_once(
        &self,
        query: &str,
        filters: &KnowledgeSearchFilters,
        major_focus: Option<&str>,
    ) -> Result<(
        Vec<FaqEvidence>,
        Vec<PolicyEvidence>,
        Vec<VectorChunkEvidence>,
    )> {
        let embedding_retrieval =
            async { Ok::<_, anyhow::Error>(self.embeddings.embed(query).await.ok()) };
        let lexical_faq_retrieval = self.db.search_faq(query, 8);
        let policy_retrieval = self.db.search_policies(query, filters, 5);
        let keyword_chunk_retrieval = self.db.search_knowledge_chunks_keyword(query, filters, 12);
        let major_chunk_retrieval =
            self.search_major_chunks_or_empty(query, filters, major_focus, 18);

        let (embedding, mut faq, policies, mut chunks, major_chunks) = tokio::try_join!(
            embedding_retrieval,
            lexical_faq_retrieval,
            policy_retrieval,
            keyword_chunk_retrieval,
            major_chunk_retrieval
        )?;
        merge_chunks(&mut chunks, major_chunks);

        if let Some(embedding) = embedding {
            let vector_faq_retrieval = async {
                self.db
                    .search_faq_vector(&embedding, faq_min_similarity(), 8)
                    .await
                    .or_else(|_| Ok::<Vec<FaqEvidence>, anyhow::Error>(Vec::new()))
            };
            let vector_chunk_retrieval = async {
                self.db
                    .search_knowledge_chunks_vector(&embedding, filters, 18)
                    .await
                    .or_else(|_| Ok::<Vec<VectorChunkEvidence>, anyhow::Error>(Vec::new()))
            };
            let (vector_faq, vector_chunks) =
                tokio::try_join!(vector_faq_retrieval, vector_chunk_retrieval)?;
            merge_faq(&mut faq, vector_faq);
            merge_chunks(&mut chunks, vector_chunks);
        }

        Ok((faq, policies, chunks))
    }

    async fn search_major_chunks_or_empty(
        &self,
        query: &str,
        filters: &KnowledgeSearchFilters,
        major_focus: Option<&str>,
        limit: i64,
    ) -> Result<Vec<VectorChunkEvidence>> {
        let Some(major_focus) = major_focus else {
            return Ok(Vec::new());
        };
        let major_name = normalize_major_for_metadata(major_focus);
        if major_name.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let topic = knowledge_topic_keyword(query);
        let chunks = self
            .db
            .search_knowledge_chunks_by_major(&major_name, topic, filters, limit)
            .await
            .or_else(|_| Ok::<Vec<VectorChunkEvidence>, anyhow::Error>(Vec::new()))?;
        if !chunks.is_empty() || topic.is_none() {
            return Ok(chunks);
        }
        self.db
            .search_knowledge_chunks_by_major(&major_name, None, filters, limit)
            .await
            .or_else(|_| Ok(Vec::new()))
    }
}

fn extract_major_focus_from_query(query: &str) -> Option<String> {
    let index = query.find("专业")?;
    let before = query[..index].trim_matches(['，', ',', '。', '？', '?', ' ']);
    if before.is_empty() || before.ends_with("这个") || before.ends_with("该") {
        return None;
    }
    let segment = before
        .split(['，', ',', '。', '？', '?', ' '])
        .next_back()
        .unwrap_or(before)
        .trim();
    let normalized = normalize_major_for_metadata(segment);
    if normalized.chars().count() >= 2 {
        Some(segment.to_owned())
    } else {
        None
    }
}

fn fallback_knowledge_queries(query: &str) -> Vec<String> {
    let compact = query
        .replace("关于", "")
        .replace("请", "")
        .replace("你", "")
        .replace("讲讲", "")
        .replace("介绍", "")
        .replace("解读", "")
        .replace("一下", "")
        .replace("呢", "")
        .replace(['？', '?', '。', '！', '!', '，', ',', ' '], "");
    let mut queries = Vec::new();
    if compact.chars().count() >= 2 && compact != query {
        queries.push(compact);
    }

    for term in [
        "公费师范生",
        "公费师范",
        "专项计划",
        "少数民族预科",
        "录取规则",
        "服从调剂",
        "同分",
        "体检",
        "语种",
        "选考科目",
        "培养方案",
        "培养目标",
        "主要课程",
        "毕业要求",
        "实践环节",
        "学分",
        "新生",
        "入学",
        "报到",
        "选课",
        "军训",
        "带电脑",
        "电脑",
        "宿舍",
        "住宿",
        "食堂",
        "校区",
    ] {
        if query.contains(term) && !queries.iter().any(|item| item == term) {
            queries.push(term.to_owned());
        }
    }
    queries
}

fn merge_faq(target: &mut Vec<FaqEvidence>, incoming: Vec<FaqEvidence>) {
    for item in incoming {
        if !target.iter().any(|existing| existing.id == item.id) {
            target.push(item);
        }
    }
}

fn grade_faq(mut faq: Vec<FaqEvidence>) -> Vec<FaqEvidence> {
    let threshold = faq_min_similarity();
    faq.retain(|item| {
        item.similarity
            .is_none_or(|similarity| similarity >= threshold || similarity >= 0.99)
    });
    faq.sort_by(|left, right| {
        right
            .similarity
            .partial_cmp(&left.similarity)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.question.cmp(&right.question))
    });
    faq.truncate(3);
    faq
}

fn faq_min_similarity() -> f64 {
    std::env::var("FAQ_MIN_SIMILARITY")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| (0.0..=1.0).contains(value))
        .unwrap_or(DEFAULT_FAQ_MIN_SIMILARITY)
}

fn major_catalog_cache_ttl() -> Duration {
    let seconds = std::env::var("MAJOR_CATALOG_CACHE_TTL_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(300);
    Duration::from_secs(seconds)
}

fn merge_policies(target: &mut Vec<PolicyEvidence>, incoming: Vec<PolicyEvidence>) {
    for item in incoming {
        if !target.iter().any(|existing| existing.id == item.id) {
            target.push(item);
        }
    }
}

fn merge_chunks(target: &mut Vec<VectorChunkEvidence>, incoming: Vec<VectorChunkEvidence>) {
    for item in incoming {
        if !target.iter().any(|existing| existing.id == item.id) {
            target.push(item);
        }
    }
}

fn resolve_score_query_subject_type(
    requested_subject_type: Option<&str>,
    records: &[AdmissionScoreRecord],
) -> Option<String> {
    if records.is_empty() {
        return requested_subject_type.map(ToOwned::to_owned);
    }
    if let Some(requested) = requested_subject_type {
        if records
            .iter()
            .any(|record| record.subject_type.as_str() == requested)
        {
            return Some(requested.to_owned());
        }
    }
    if records.iter().all(|record| record.subject_type == "未区分") {
        return Some("未区分".to_owned());
    }
    requested_subject_type.map(ToOwned::to_owned)
}

fn score_query_subject_diagnostics(
    requested_subject_type: Option<&str>,
    resolved_subject_type: Option<&str>,
) -> Option<serde_json::Value> {
    let requested = requested_subject_type?;
    let resolved = resolved_subject_type?;
    if requested == resolved {
        return None;
    }
    Some(json!({
        "requestedSubjectType": requested,
        "actualSubjectType": resolved,
        "note": "用户指定科类没有单列记录，已使用录取统计表中的实际科类记录。"
    }))
}

fn sanitize_major_focus_for_retrieval(value: &str) -> Option<String> {
    let mut text = value.trim().to_owned();
    for marker in [
        "培养方案",
        "培养目标",
        "毕业要求",
        "毕业条件",
        "毕业需要",
        "主要课程",
        "核心课程",
        "课程",
        "教育实习",
        "专业实践",
        "实践环节",
        "第二课堂",
        "创新实践",
        "毕业创作",
        "毕业论文",
        "学分",
        "适合",
        "匹配",
        "契合",
        "有没有",
        "怎么安排",
        "是什么",
    ] {
        if let Some(index) = text.find(marker) {
            text.truncate(index);
        }
    }
    let text = text
        .replace("这个专业", "")
        .replace("该专业", "")
        .replace("专业的", "")
        .replace("专业里", "")
        .replace("专业", "")
        .trim_matches([
            '，', ',', '。', '？', '?', '！', '!', ' ', '的', '里', '：', ':',
        ])
        .to_owned();
    let text = text
        .split_whitespace()
        .next()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&text)
        .to_owned();
    (normalize_major_for_metadata(&text).chars().count() >= 2).then_some(text)
}

fn training_plan_major_fallback_queries(query: &str, major_focus: Option<&str>) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(major) = major_focus.and_then(sanitize_major_focus_for_retrieval) {
        candidates.push(major);
    }
    if let Some(major) = extract_major_focus_from_query(query) {
        candidates.push(major);
    }
    let mut before_marker = query.to_owned();
    for marker in [
        "培养方案",
        "培养目标",
        "毕业要求",
        "毕业条件",
        "毕业需要",
        "主要课程",
        "核心课程",
        "课程",
        "教育实习",
        "专业实践",
        "实践环节",
        "第二课堂",
        "创新实践",
        "毕业创作",
        "毕业论文",
        "学分",
        "有没有",
        "怎么安排",
        "是什么",
    ] {
        if let Some(index) = before_marker.find(marker) {
            before_marker.truncate(index);
        }
    }
    if let Some(before_marker) = sanitize_major_focus_for_retrieval(&before_marker) {
        candidates.push(before_marker);
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn query_looks_like_training_plan(query: &str) -> bool {
    contains_any(
        query,
        &[
            "培养方案",
            "培养目标",
            "毕业要求",
            "毕业条件",
            "毕业需要",
            "主要课程",
            "核心课程",
            "课程",
            "教育实习",
            "专业实践",
            "实践环节",
            "第二课堂",
            "创新实践",
            "毕业创作",
            "毕业论文",
            "学分",
        ],
    )
}

#[derive(Debug, Clone)]
pub struct KnowledgeRetrievalResult {
    pub structured_result: ChatStructuredResult,
    pub citations: Vec<ChatCitation>,
}

pub fn route_message(message: &str) -> RouteDecision {
    let trimmed = message.trim();
    if is_greeting(trimmed) {
        return RouteDecision {
            intent: RetrievalIntent::Greeting,
            must_use_tools: false,
            reason: "用户寒暄，不需要查数据。".to_owned(),
        };
    }
    if is_probability_message(trimmed) {
        return RouteDecision {
            intent: RetrievalIntent::ProbabilityAssessment,
            must_use_tools: true,
            reason: "录取概率必须基于真实录取统计。".to_owned(),
        };
    }
    if is_score_message(trimmed) {
        return RouteDecision {
            intent: RetrievalIntent::ScoreQuery,
            must_use_tools: true,
            reason: "分数线问题必须查询录取统计表。".to_owned(),
        };
    }
    if is_knowledge_message(trimmed) {
        return RouteDecision {
            intent: RetrievalIntent::KnowledgeAnswer,
            must_use_tools: true,
            reason: "招生简章、培养方案、FAQ、学院专业目录必须查工具或知识库。".to_owned(),
        };
    }
    RouteDecision {
        intent: RetrievalIntent::GeneralAnswer,
        must_use_tools: false,
        reason: "普通对话，不需要事实工具。".to_owned(),
    }
}

pub fn infer_knowledge_filters(query: &str) -> KnowledgeSearchFilters {
    let mut filters = KnowledgeSearchFilters {
        category: None,
        year: extract_year(query),
        document_kind: None,
    };

    if contains_any(
        query,
        &[
            "招生简章",
            "招生章程",
            "招生计划",
            "招生专业",
            "招生网站",
            "招生咨询",
            "咨询电话",
            "联系电话",
            "录取规则",
            "专业级差",
            "级差",
            "服从调剂",
            "退档",
            "同分",
            "优先录取",
            "分数相同",
            "成绩相同",
            "体检",
            "语种",
            "外语语种",
            "单科成绩",
            "选考",
            "选科",
        ],
    ) {
        filters.category = Some("招生简章".to_owned());
        filters.document_kind = Some("admission_brochure".to_owned());
        return filters;
    }

    if contains_any(query, &["学校介绍", "学校简介", "学校情况", "院校介绍"])
        || (contains_any(query, &["介绍", "简介", "讲讲", "说说", "了解"])
            && contains_any(query, &["学校", "院校", "哈师大", "哈尔滨师范大学", "大学"])
            && !contains_any(
                query,
                &[
                    "专业",
                    "学院",
                    "课程",
                    "培养",
                    "录取",
                    "分数",
                    "位次",
                    "招生计划",
                ],
            ))
    {
        filters.category = Some("招生简章".to_owned());
        filters.document_kind = Some("admission_brochure".to_owned());
        return filters;
    }

    if contains_any(
        query,
        &[
            "培养方案",
            "培养目标",
            "主要课程",
            "课程",
            "学分",
            "毕业要求",
            "实践环节",
            "学院简介",
            "学院介绍",
            "有哪些专业",
            "有什么专业",
            "开设哪些专业",
        ],
    ) || query.contains("学院") && query.contains("专业")
        || query.contains("选考")
        || query.contains("选科")
    {
        filters.category = Some("培养方案".to_owned());
        filters.document_kind = Some("training_plan".to_owned());
    }

    filters
}

pub fn extract_college_major_catalog_query(message: &str) -> Option<String> {
    let trimmed = message.trim();
    if !contains_any(
        trimmed,
        &[
            "有哪些专业",
            "有什么专业",
            "有啥专业",
            "开设哪些专业",
            "设置哪些专业",
            "包括哪些专业",
        ],
    ) {
        return None;
    }

    let marker = "学院";
    let end = trimmed.find(marker)? + marker.len();
    let before = &trimmed[..end];
    let college = before
        .split(['，', ',', '。', '？', '?', '！', '!', ' '])
        .next_back()
        .unwrap_or(before)
        .trim();
    if college.len() >= marker.len()
        && !college.contains("哈尔滨师范大学")
        && !college.contains("哈师大")
    {
        Some(college.to_owned())
    } else {
        None
    }
}

pub fn render_college_major_answer(
    college_name: &str,
    majors: &[CollegeMajorRecord],
) -> (String, ChatStructuredResult, Vec<ChatCitation>) {
    if majors.is_empty() {
        let reply = format!(
            "我按已入库的 2025 版培养方案查了一下，暂时没有按“{college_name}”整理到专业列表。你可以换成学院全称，或直接问某个专业的培养方案、课程设置、毕业要求。"
        );
        return (
            reply,
            ChatStructuredResult::KnowledgeAnswer {
                query: format!("{college_name}有哪些专业"),
                faq: Vec::new(),
                policies: Vec::new(),
                vector_chunks: Vec::new(),
            },
            Vec::new(),
        );
    }

    let major_names = majors
        .iter()
        .map(|item| item.major_name.as_str())
        .collect::<Vec<_>>();
    let reply = format!(
        "可以查到，我按已入库的 2025 版专业培养方案帮你整理了一下：{college_name}相关专业有 {}。\n\n这里整理的是培养方案中的学院归属和专业信息；具体某一年、某个省份是否招生、招生计划是多少，还要以当年分省招生计划和省级招生主管部门公布的信息为准。\n\n你如果对其中某个专业感兴趣，我可以继续帮你看培养目标、主要课程、学分结构、毕业要求，也可以结合省份和分数查近几年录取情况。",
        major_names.join("、")
    );
    let structured_result = ChatStructuredResult::KnowledgeAnswer {
        query: format!("{college_name}有哪些专业"),
        faq: Vec::new(),
        policies: Vec::new(),
        vector_chunks: vec![VectorChunkEvidence {
            id: format!("college-training-plan-majors:{college_name}"),
            title: Some(format!("2025版{college_name}专业培养方案目录")),
            content: format!(
                "学院：{college_name}\n年份：2025\n覆盖专业：{}。",
                major_names.join("、")
            ),
            category: Some("培养方案".to_owned()),
            year: Some(2025),
            similarity: Some(1.0),
            metadata: json!({
                "documentKind": "training_plan",
                "college": college_name,
                "majorNames": major_names,
                "contentType": "college_major_catalog"
            }),
        }],
    };
    let citations = vec![ChatCitation {
        year: Some(2025),
        source_label: format!("2025版{college_name}专业培养方案"),
        source_url: None,
    }];

    (reply, structured_result, citations)
}

pub fn render_knowledge_answer(result: &ChatStructuredResult) -> String {
    let ChatStructuredResult::KnowledgeAnswer {
        faq,
        policies,
        vector_chunks,
        ..
    } = result
    else {
        return "我已经整理好相关信息。".to_owned();
    };

    if let Some(faq) = faq.first() {
        return faq.answer.clone();
    }
    if let Some(policy) = policies.first() {
        return format!(
            "我查到相关政策《{}》。你可以继续问具体条款，我会结合已收录内容帮你解释。",
            policy.title
        );
    }
    if let Some(chunk) = vector_chunks.first() {
        let title = chunk.title.as_deref().unwrap_or("相关资料");
        let excerpt = compact_excerpt(&chunk.content, 260);
        return format!("我查到《{title}》里有相关内容，核心信息是：{excerpt}");
    }
    "我按已入库的招生简章、培养方案和 FAQ 查了一下，暂时没有找到可以直接支撑回答的资料。你可以补充具体专业、学院、年份或省份，我再帮你精确查。".to_owned()
}

pub fn render_major_disambiguation(
    query: &str,
    candidates: Vec<MajorCandidate>,
    pending_intent: ChatIntent,
) -> ChatStructuredResult {
    let missing_fields = if candidates.len() == 1 {
        Vec::new()
    } else {
        vec!["major".to_owned()]
    };
    ChatStructuredResult::MajorDisambiguation {
        query: query.to_owned(),
        pending_intent,
        candidates,
        missing_fields,
        message: "需要先确认具体专业后再继续。".to_owned(),
    }
}

fn is_greeting(message: &str) -> bool {
    let normalized = normalize_text(message);
    matches!(
        normalized.as_str(),
        "你好" | "您好" | "哈喽" | "哈啰" | "hello" | "hi" | "嗨"
    ) || contains_any(
        message,
        &[
            "你是谁",
            "你是啥",
            "你是什么",
            "你能做什么",
            "你可以做什么",
            "你会什么",
            "你擅长什么",
            "你擅长",
            "介绍一下你",
            "介绍下你",
            "自我介绍",
        ],
    ) || (contains_any(
        message,
        &["你好", "您好", "哈喽", "哈啰", "hello", "hi", "嗨"],
    ) && message.chars().count() <= 20
        && contains_any(message, &["谁", "什么", "做什么", "会什么"]))
}

fn is_probability_message(message: &str) -> bool {
    contains_any(
        message,
        &[
            "能上",
            "能不能上",
            "能报",
            "能录取",
            "能不能录取",
            "录取概率",
            "概率",
            "稳吗",
            "稳不稳",
            "冲稳保",
            "有希望",
            "希望吗",
            "希望大吗",
            "希望大不大",
            "把握",
            "录取机会",
            "风险",
        ],
    )
}

fn is_score_message(message: &str) -> bool {
    contains_any(
        message,
        &[
            "录取线",
            "录取分数",
            "最低分",
            "分数线",
            "位次",
            "排名",
            "近三年",
            "历年分数",
        ],
    )
}

fn is_knowledge_message(message: &str) -> bool {
    let asks_school_intro = contains_any(message, &["介绍", "讲讲", "说说"])
        && contains_any(message, &["学校", "院校", "哈师大", "哈尔滨师范大学"]);
    let asks_major_fit = contains_any(message, &["适合", "匹配", "契合", "合适"])
        && contains_any(
            message,
            &["专业", "培养", "老师", "教师", "实验", "课程", "就业"],
        );
    contains_any(
        message,
        &[
            "招生简章",
            "招生章程",
            "录取规则",
            "优先录取",
            "分数相同",
            "成绩相同",
            "调剂",
            "体检",
            "语种",
            "招生网站",
            "招生咨询",
            "咨询电话",
            "联系电话",
            "普通类",
            "艺术类",
            "综合分",
            "校园",
            "校园生活",
            "大学生活",
            "学生生活",
            "新生",
            "入学",
            "报到",
            "选课",
            "军训",
            "带电脑",
            "电脑",
            "校区",
            "学校介绍",
            "学校情况",
            "学校简介",
            "院校介绍",
            "学校优势",
            "优势专业",
            "住宿",
            "宿舍",
            "食堂",
            "社团",
            "就业",
            "升学",
            "能直接比较",
            "公费师范",
            "公费师范生",
            "专项计划",
            "少数民族预科",
            "专业级差",
            "单科成绩",
            "培养方案",
            "培养目标",
            "课程",
            "学分",
            "毕业要求",
            "毕业条件",
            "毕业需要",
            "实践环节",
            "教育实习",
            "专业实践",
            "创新实践",
            "第二课堂",
            "毕业创作",
            "毕业论文",
            "学院简介",
            "学院介绍",
            "有哪些专业",
            "有什么专业",
            "开设哪些专业",
            "选考科目",
            "选考",
            "选科",
            "FAQ",
        ],
    ) || asks_school_intro
        || asks_major_fit
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn extract_year(query: &str) -> Option<i32> {
    for year in 2021..=2039 {
        if query.contains(&year.to_string()) {
            return Some(year);
        }
    }
    None
}

fn normalize_text(text: &str) -> String {
    text.to_lowercase()
        .replace(
            [
                ' ', '\n', '\t', '（', '）', '(', ')', '，', ',', '。', '？', '?', '、', ':', '：',
            ],
            "",
        )
        .trim_end_matches("专业")
        .to_owned()
}

fn root_major_name(text: &str) -> String {
    text.split(['（', '('])
        .next()
        .map(normalize_text)
        .unwrap_or_else(|| normalize_text(text))
}

fn direction_tokens(query: &str) -> Vec<&'static str> {
    const GROUPS: &[&[&str]] = &[
        &[
            "计算机",
            "软件",
            "数据",
            "物联网",
            "数字媒体技术",
            "电子信息",
        ],
        &["数学", "数学与应用数学", "统计", "数据科学"],
        &["物理", "物理学"],
        &["化学", "应用化学", "材料化学"],
        &["生物", "生物科学", "生物技术"],
        &["历史", "历史学", "文物保护"],
        &["政治", "思想政治教育", "马克思主义"],
        &[
            "英语",
            "外语",
            "翻译",
            "商务英语",
            "俄语",
            "日语",
            "西班牙语",
            "法语",
        ],
        &["中文", "汉语言", "汉语言文学", "汉语国际教育"],
        &["小学教育", "学前教育", "教育技术", "心理学", "应用心理"],
        &["音乐", "舞蹈", "作曲", "表演"],
        &["美术", "绘画", "书法", "设计", "视觉传达"],
        &["地理", "地理科学", "地理信息", "人文地理"],
    ];
    let normalized = normalize_text(query);
    GROUPS
        .iter()
        .filter(|group| {
            group
                .iter()
                .any(|token| normalized.contains(&normalize_text(token)))
        })
        .flat_map(|group| group.iter().copied())
        .collect()
}

fn score_major_candidate(query: &str, major: &MajorRecord) -> i32 {
    let normalized_query = normalize_text(query);
    let normalized_name = normalize_text(&major.name);
    let root_name = root_major_name(&major.name);
    let mut score = 0;
    if normalized_name == normalized_query {
        score += 120;
    }
    if normalized_name.contains(&normalized_query) || normalized_query.contains(&normalized_name) {
        score += 70;
    }
    if !root_name.is_empty()
        && (normalized_query.contains(&root_name) || root_name.contains(&normalized_query))
    {
        score += 110;
        score += ((root_name.chars().count() as i32) * 6).min(90);
    }
    for token in direction_tokens(query) {
        if normalized_name.contains(&normalize_text(token)) {
            score += 35;
        }
    }
    if query.contains("师范") && major.is_normal_major {
        score += 12;
    }
    for variant in [
        "固边",
        "公费",
        "优师",
        "定向",
        "实验班",
        "行知",
        "专项",
        "少数民族",
        "非遗传承",
    ] {
        if major.name.contains(variant) && !query.contains(variant) {
            score -= 12;
        }
    }
    score
}

fn grade_chunks(
    mut chunks: Vec<VectorChunkEvidence>,
    query: &str,
    major_focus: Option<&str>,
    strict_major_focus: bool,
) -> Vec<VectorChunkEvidence> {
    if let Some(major_focus) = major_focus {
        let focused_chunks = chunks
            .iter()
            .filter(|chunk| chunk_matches_major_focus(chunk, major_focus))
            .cloned()
            .collect::<Vec<_>>();
        if !focused_chunks.is_empty() {
            chunks = focused_chunks;
        } else if strict_major_focus {
            return Vec::new();
        }
    }

    chunks.sort_by(|left, right| {
        score_chunk(right, query, major_focus)
            .partial_cmp(&score_chunk(left, query, major_focus))
            .unwrap_or(Ordering::Equal)
    });
    chunks.truncate(8);
    chunks
}

fn score_chunk(chunk: &VectorChunkEvidence, query: &str, major_focus: Option<&str>) -> f64 {
    let mut score = chunk.similarity.unwrap_or(0.35);
    let compact = chunk.content.replace(char::is_whitespace, "");
    score += (compact
        .chars()
        .filter(|ch| ('\u{4e00}'..='\u{9fff}').contains(ch))
        .count() as f64
        / 4000.0)
        .min(0.04);
    score += lexical_overlap_score(chunk, query).min(0.18);
    for token in [
        "录取规则",
        "招生计划",
        "培养目标",
        "毕业要求",
        "课程",
        "学分",
        "实践教学",
    ] {
        if chunk.content.contains(token) {
            score += 0.03;
        }
    }
    if let Some(college) = extract_college_major_catalog_query(query) {
        if chunk
            .metadata
            .get("college")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value != college)
        {
            score -= 0.4;
        }
    }
    if let Some(major_focus) = major_focus {
        let expected = normalize_major_for_metadata(major_focus);
        if let Some(chunk_major) = chunk
            .metadata
            .get("majorName")
            .and_then(|value| value.as_str())
        {
            let actual = normalize_major_for_metadata(chunk_major);
            if !expected.is_empty() && !actual.is_empty() {
                if major_metadata_matches(major_focus, chunk_major) {
                    score += 0.55;
                } else {
                    score -= 0.65;
                }
            }
        }
    }
    if let Some(section_type) = chunk
        .metadata
        .get("sectionType")
        .and_then(|value| value.as_str())
    {
        if contains_any(query, &["毕业条件", "毕业需要", "学分"])
            && section_type == "graduation_conditions"
        {
            score += 0.45;
        }
        if contains_any(query, &["培养目标"]) && section_type == "training_objectives" {
            score += 0.45;
        }
        if contains_any(query, &["实践环节", "教育实习", "第二课堂", "创新实践"])
            && matches!(section_type, "practice_teaching" | "semester_weeks")
        {
            score += 0.35;
        }
        if contains_any(query, &["主要课程", "课程有没有", "课程有哪些"])
            && section_type == "teaching_plan"
        {
            score += 0.25;
        }
    }
    if chunk.content.contains("目录") || chunk.content.contains("......") {
        score -= 0.08;
    }
    score
}

fn chunk_matches_major_focus(chunk: &VectorChunkEvidence, major_focus: &str) -> bool {
    let expected = normalize_major_for_metadata(major_focus);
    if expected.is_empty() {
        return false;
    }

    if let Some(chunk_major) = chunk
        .metadata
        .get("majorName")
        .and_then(|value| value.as_str())
    {
        return major_metadata_matches(major_focus, chunk_major);
    }

    let title = normalize_major_for_metadata(chunk.title.as_deref().unwrap_or_default());
    if title.contains(&expected) {
        return true;
    }

    let content_head = chunk.content.chars().take(240).collect::<String>();
    normalize_major_for_metadata(&content_head).contains(&expected)
}

fn major_metadata_matches(expected_major: &str, actual_major: &str) -> bool {
    let expected = normalize_major_for_metadata(expected_major);
    let actual = normalize_major_for_metadata(actual_major);
    if expected.is_empty() || actual.is_empty() {
        return false;
    }
    if !(expected == actual || expected.contains(&actual) || actual.contains(&expected)) {
        return false;
    }
    compatible_major_variants(expected_major, actual_major)
}

fn compatible_major_variants(expected_major: &str, actual_major: &str) -> bool {
    let expected = normalize_text(expected_major);
    let actual = normalize_text(actual_major);
    for variant in ["行知", "实验班", "非师范"] {
        let expected_has = expected.contains(variant);
        let actual_has = actual.contains(variant);
        if expected_has != actual_has {
            return false;
        }
    }
    if expected.contains("师范") != actual.contains("师范") {
        let expected_has_specific_variant =
            expected.contains("非师范") || expected.contains("行知") || expected.contains("实验班");
        let actual_has_specific_variant =
            actual.contains("非师范") || actual.contains("行知") || actual.contains("实验班");
        if expected_has_specific_variant || actual_has_specific_variant {
            return false;
        }
    }
    true
}

fn normalize_major_for_metadata(text: &str) -> String {
    root_major_name(text)
        .replace("师范类", "")
        .replace("师范", "")
        .replace("专业", "")
}

fn knowledge_topic_keyword(query: &str) -> Option<&'static str> {
    for keyword in [
        "毕业要求",
        "毕业条件",
        "培养目标",
        "主要课程",
        "课程",
        "实践环节",
        "教育实习",
        "毕业创作",
        "第二课堂",
        "创新实践",
        "学分",
    ] {
        if query.contains(keyword) {
            return Some(keyword);
        }
    }
    None
}

fn lexical_overlap_score(chunk: &VectorChunkEvidence, query: &str) -> f64 {
    let haystack = normalize_text(&format!(
        "{}{}{}",
        chunk.title.as_deref().unwrap_or_default(),
        chunk.content,
        chunk.metadata
    ));
    meaningful_query_terms(query)
        .into_iter()
        .filter(|term| haystack.contains(term))
        .map(|term| (term.chars().count() as f64 / 100.0).min(0.05))
        .sum()
}

fn meaningful_query_terms(query: &str) -> Vec<String> {
    let normalized = normalize_text(query);
    let chars = normalized.chars().collect::<Vec<_>>();
    let mut terms = Vec::new();
    for len in (2..=8).rev() {
        if chars.len() < len {
            continue;
        }
        for window in chars.windows(len) {
            let term = window.iter().collect::<String>();
            if !is_low_value_query_term(&term)
                && !terms.iter().any(|existing: &String| existing == &term)
            {
                terms.push(term);
            }
        }
    }
    terms.truncate(24);
    terms
}

fn is_low_value_query_term(term: &str) -> bool {
    [
        "是什么",
        "有没有",
        "一下",
        "介绍",
        "讲讲",
        "专业",
        "课程",
        "培养",
        "方案",
        "招生",
        "录取",
        "可以",
        "怎么",
        "哪些",
        "什么",
    ]
    .iter()
    .any(|noise| term == *noise || term.starts_with(noise) || term.ends_with(noise))
}

fn build_knowledge_citations(
    faq: &[FaqEvidence],
    policies: &[PolicyEvidence],
    chunks: &[VectorChunkEvidence],
) -> Vec<ChatCitation> {
    let mut citations = Vec::new();
    citations.extend(faq.iter().take(2).map(|item| ChatCitation {
        year: None,
        source_label: item.source_label.clone(),
        source_url: None,
    }));
    citations.extend(policies.iter().take(1).map(|item| ChatCitation {
        year: item.year,
        source_label: item.title.clone(),
        source_url: item.source_url.clone(),
    }));
    citations.extend(chunks.iter().take(2).map(|item| ChatCitation {
        year: item.year,
        source_label: item.title.clone().unwrap_or_else(|| "知识库".to_owned()),
        source_url: None,
    }));
    citations
}

fn compact_excerpt(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    normalized.chars().take(max_chars).collect::<String>() + "..."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_factual_questions_to_tools() {
        let score = route_message("河北历史类汉语言文学近三年分数线");
        assert_eq!(score.intent, RetrievalIntent::ScoreQuery);
        assert!(score.must_use_tools);

        let knowledge = route_message("音乐学院有哪些专业");
        assert_eq!(knowledge.intent, RetrievalIntent::KnowledgeAnswer);
        assert!(knowledge.must_use_tools);
    }

    #[test]
    fn routes_identity_questions_to_greeting() {
        let identity = route_message("你好 你是谁");
        assert_eq!(identity.intent, RetrievalIntent::Greeting);
        assert!(!identity.must_use_tools);

        let capability = route_message("你能做什么？");
        assert_eq!(capability.intent, RetrievalIntent::Greeting);
        assert!(!capability.must_use_tools);

        let strengths = route_message("你擅长什么？");
        assert_eq!(strengths.intent, RetrievalIntent::Greeting);
        assert!(!strengths.must_use_tools);
    }

    #[test]
    fn routes_campus_intro_to_knowledge() {
        let campus = route_message("介绍一下校园？");
        assert_eq!(campus.intent, RetrievalIntent::KnowledgeAnswer);
        assert!(campus.must_use_tools);

        let school = route_message("介绍一下学校");
        assert_eq!(school.intent, RetrievalIntent::KnowledgeAnswer);
        assert!(school.must_use_tools);

        let dorm = route_message("学校住宿条件怎么样？");
        assert_eq!(dorm.intent, RetrievalIntent::KnowledgeAnswer);
        assert!(dorm.must_use_tools);

        let practice = route_message("音乐学专业的教育实习和实践环节怎么安排？");
        assert_eq!(practice.intent, RetrievalIntent::KnowledgeAnswer);
        assert!(practice.must_use_tools);

        let same_score = route_message("如果两个考生投档成绩一样，学校会优先录取谁？");
        assert_eq!(same_score.intent, RetrievalIntent::KnowledgeAnswer);
        assert!(same_score.must_use_tools);

        let new_student = route_message("入学前需要自己选课吗？军训大概多久？新生可以带电脑吗？");
        assert_eq!(new_student.intent, RetrievalIntent::KnowledgeAnswer);
        assert!(new_student.must_use_tools);
    }

    #[test]
    fn routes_natural_probability_and_major_fit_questions() {
        let probability = route_message("河北500分报汉语言文学师范类有希望吗？");
        assert_eq!(probability.intent, RetrievalIntent::ProbabilityAssessment);
        assert!(probability.must_use_tools);

        let fit = route_message("生物科学专业适合喜欢实验、以后想当老师的学生吗？");
        assert_eq!(fit.intent, RetrievalIntent::KnowledgeAnswer);
        assert!(fit.must_use_tools);
    }

    #[test]
    fn filters_low_similarity_faq_before_context() {
        let faq = grade_faq(vec![
            FaqEvidence {
                id: "exact".to_owned(),
                question: "学校的招生咨询电话是多少？".to_owned(),
                answer: "0451-88067377".to_owned(),
                category: "志愿填报".to_owned(),
                source_label: "FAQ".to_owned(),
                similarity: Some(1.0),
            },
            FaqEvidence {
                id: "weak".to_owned(),
                question: "学校的招生代码是多少？".to_owned(),
                answer: "10231".to_owned(),
                category: "志愿填报".to_owned(),
                source_label: "FAQ".to_owned(),
                similarity: Some(0.60),
            },
        ]);

        assert_eq!(faq.len(), 1);
        assert_eq!(faq[0].id, "exact");
    }

    #[test]
    fn training_plan_major_matching_respects_variants() {
        assert!(major_metadata_matches(
            "音乐学专业（师范）",
            "音乐学专业（师范）"
        ));
        assert!(!major_metadata_matches(
            "音乐学专业（师范）",
            "音乐学专业（非师范）"
        ));
        assert!(!major_metadata_matches(
            "计算机科学与技术专业（师范）",
            "计算机科学与技术专业（行知班）"
        ));
        assert!(major_metadata_matches("生物科学（师范类）", "生物科学专业"));
        assert!(!major_metadata_matches(
            "计算机科学与技术专业（师范）",
            "计算机科学与技术专业（非师范）"
        ));
        assert!(major_metadata_matches("生物科学专业", "生物科学专业"));
        assert_eq!(
            normalize_major_for_metadata("计算机科学与技术（师范）"),
            "计算机科学与技术"
        );
        assert_eq!(
            extract_major_focus_from_query("生物科学专业 如果喜欢实验，这个专业培养目标适合吗？")
                .as_deref(),
            Some("生物科学")
        );
        assert_eq!(
            sanitize_major_focus_for_retrieval("地理科学专业培养目标是什么？").as_deref(),
            Some("地理科学")
        );
        assert_eq!(
            sanitize_major_focus_for_retrieval("地理信息科学 GIS和遥感相关课程有没有？").as_deref(),
            Some("地理信息科学")
        );
        assert_eq!(
            extract_major_focus_from_query("这个专业培养目标是什么？"),
            None
        );
    }

    #[test]
    fn extracts_college_major_catalog_query() {
        assert_eq!(
            extract_college_major_catalog_query("教育科学学院还有哪些专业"),
            Some("教育科学学院".to_owned())
        );
        assert_eq!(
            extract_college_major_catalog_query("音乐学院有哪些专业"),
            Some("音乐学院".to_owned())
        );
    }

    #[test]
    fn infers_training_plan_filter_for_college_majors() {
        let filters = infer_knowledge_filters("音乐学院有哪些专业");
        assert_eq!(filters.category, Some("培养方案".to_owned()));
        assert_eq!(filters.document_kind, Some("training_plan".to_owned()));
    }

    #[test]
    fn infers_admission_brochure_filter_for_school_intro() {
        let filters = infer_knowledge_filters("简单介绍一下学校");
        assert_eq!(filters.category, Some("招生简章".to_owned()));
        assert_eq!(filters.document_kind, Some("admission_brochure".to_owned()));
    }

    #[test]
    fn major_scoring_prefers_explicit_discipline_over_generic_normal_major() {
        let query = "辽宁物理类584分物理学师范类录取概率";
        let physics = MajorRecord {
            slug: "physics".to_owned(),
            name: "物理学（师范类）".to_owned(),
            code: None,
            is_normal_major: true,
            latest_score: None,
        };
        let preschool = MajorRecord {
            slug: "preschool".to_owned(),
            name: "学前教育（固边计划，师范类）".to_owned(),
            code: None,
            is_normal_major: true,
            latest_score: None,
        };

        assert!(score_major_candidate(query, &physics) > score_major_candidate(query, &preschool));
    }

    #[test]
    fn major_scoring_keeps_major_stronger_than_subject_type() {
        let query = "河北历史类500分汉语言文学师范类稳吗";
        let literature = MajorRecord {
            slug: "literature".to_owned(),
            name: "汉语言文学（师范类）".to_owned(),
            code: None,
            is_normal_major: true,
            latest_score: None,
        };
        let history = MajorRecord {
            slug: "history".to_owned(),
            name: "历史学（师范类）".to_owned(),
            code: None,
            is_normal_major: true,
            latest_score: None,
        };

        assert!(score_major_candidate(query, &literature) > score_major_candidate(query, &history));
    }

    #[test]
    fn major_scoring_prefers_long_exact_major_over_contained_short_major() {
        let query = "山东数据科学与大数据技术2021到2025录取线";
        let full = MajorRecord {
            slug: "data-tech".to_owned(),
            name: "数据科学与大数据技术".to_owned(),
            code: None,
            is_normal_major: false,
            latest_score: None,
        };
        let short = MajorRecord {
            slug: "data-science".to_owned(),
            name: "数据科学".to_owned(),
            code: None,
            is_normal_major: false,
            latest_score: None,
        };

        assert!(score_major_candidate(query, &full) > score_major_candidate(query, &short));
    }
}
