use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use domain::{
    AdmissionScoreRecord, ChatCitation, ChatStructuredResult, ConversationHistory,
    ConversationMessage, FaqEvidence, LatestScore, MajorAdmissionProvince, PolicyEvidence,
    ProvinceAdmissionMajor, ResolvedMemory, ScoreSummary, VectorChunkEvidence,
};
use serde_json::Value;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::time::Duration;

#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

#[derive(Debug, Clone)]
pub struct ProvinceRecord {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct MajorRecord {
    pub slug: String,
    pub name: String,
    pub code: Option<String>,
    pub is_normal_major: bool,
    pub latest_score: Option<LatestScore>,
}

#[derive(Debug, Clone)]
pub struct CollegeMajorRecord {
    pub college_name: String,
    pub major_name: String,
    pub title: Option<String>,
    pub chunk_count: i64,
}

#[derive(Debug, Clone)]
pub struct KnowledgeSearchFilters {
    pub category: Option<String>,
    pub year: Option<i32>,
    pub document_kind: Option<String>,
}

impl Database {
    pub fn connect_lazy(database_url: &str) -> Result<Self> {
        let max_connections = read_env_u32("DATABASE_MAX_CONNECTIONS", 24);
        let min_connections = read_env_u32("DATABASE_MIN_CONNECTIONS", 1).min(max_connections);
        let acquire_timeout_secs = read_env_u64("DATABASE_ACQUIRE_TIMEOUT_SECS", 5);

        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .min_connections(min_connections)
            .acquire_timeout(Duration::from_secs(acquire_timeout_secs))
            .connect_lazy(database_url)
            .context("failed to create lazy postgres pool")?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn health_check(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    pub async fn resolve_province(&self, value: &str) -> Result<Option<ProvinceRecord>> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let row = sqlx::query(
            r#"
            SELECT code, name
            FROM provinces
            WHERE code = $1 OR name = $1 OR name = $2
            LIMIT 1
            "#,
        )
        .bind(trimmed)
        .bind(trimmed.trim_end_matches('省'))
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| ProvinceRecord {
            code: row.get("code"),
            name: row.get("name"),
        }))
    }

    pub async fn list_major_catalog(&self) -> Result<Vec<MajorRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT slug, name, code, is_normal_major
            FROM majors
            ORDER BY name
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| MajorRecord {
                slug: row.get("slug"),
                name: row.get("name"),
                code: row.try_get("code").ok(),
                is_normal_major: row.get("is_normal_major"),
                latest_score: None,
            })
            .collect())
    }

    pub async fn list_major_catalog_with_latest_scores(&self) -> Result<Vec<MajorRecord>> {
        let rows = sqlx::query(
            r#"
            WITH latest_scores AS (
              SELECT DISTINCT ON (m.slug)
                m.slug,
                s.year AS latest_year,
                s.min_score AS latest_min_score
              FROM admission_scores s
              JOIN majors m ON m.id = s.major_id
              WHERE s.batch NOT ILIKE '%专升本%'
                AND s.batch NOT ILIKE '%单招%'
                AND s.batch NOT ILIKE '%预科%'
              ORDER BY m.slug, s.year DESC, s.min_score ASC
            )
            SELECT
              m.slug,
              m.name,
              m.code,
              m.is_normal_major,
              ls.latest_year,
              ls.latest_min_score
            FROM majors m
            LEFT JOIN latest_scores ls ON ls.slug = m.slug
            ORDER BY m.name
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| {
                let latest_year = row.try_get::<Option<i32>, _>("latest_year").ok().flatten();
                let latest_min_score = row
                    .try_get::<Option<i32>, _>("latest_min_score")
                    .ok()
                    .flatten();
                MajorRecord {
                    slug: row.get("slug"),
                    name: row.get("name"),
                    code: row.try_get("code").ok(),
                    is_normal_major: row.get("is_normal_major"),
                    latest_score: latest_year
                        .zip(latest_min_score)
                        .map(|(year, min_score)| LatestScore { year, min_score }),
                }
            })
            .collect())
    }

    pub async fn latest_score_for_major(&self, major_slug: &str) -> Result<Option<LatestScore>> {
        let row = sqlx::query(
            r#"
            SELECT s.year, s.min_score
            FROM admission_scores s
            JOIN majors m ON m.id = s.major_id
            WHERE m.slug = $1
              AND s.batch NOT ILIKE '%专升本%'
              AND s.batch NOT ILIKE '%单招%'
              AND s.batch NOT ILIKE '%预科%'
            ORDER BY s.year DESC, s.min_score ASC
            LIMIT 1
            "#,
        )
        .bind(major_slug)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| LatestScore {
            year: row.get("year"),
            min_score: row.get("min_score"),
        }))
    }

    pub async fn query_admission_scores(
        &self,
        province: &str,
        major_slug: &str,
        subject_type: Option<&str>,
        year: Option<i32>,
    ) -> Result<Vec<AdmissionScoreRecord>> {
        let province_value = province.trim();
        let rows = sqlx::query(
            r#"
            SELECT
              s.year, s.batch, s.subject_type, s.admitted_count,
              s.min_score, s.avg_score, s.max_score, s.min_rank,
              s.source_label, s.source_url
            FROM admission_scores s
            JOIN majors m ON m.id = s.major_id
            JOIN provinces p ON p.id = s.province_id
            WHERE (p.code = $1 OR p.name = $1)
              AND (m.slug = $2 OR m.name = $2)
              AND ($3::text IS NULL OR s.subject_type = $3 OR s.subject_type = '未区分')
              AND ($4::int IS NULL OR s.year = $4)
              AND s.batch NOT ILIKE '%专升本%'
              AND s.batch NOT ILIKE '%单招%'
              AND s.batch NOT ILIKE '%预科%'
            ORDER BY s.year DESC, s.batch, s.subject_type, s.min_score
            LIMIT 200
            "#,
        )
        .bind(province_value)
        .bind(major_slug)
        .bind(subject_type)
        .bind(year)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| AdmissionScoreRecord {
                year: row.get("year"),
                batch: row.get("batch"),
                subject_type: row.get("subject_type"),
                admitted_count: row.try_get("admitted_count").ok(),
                min_score: row.get("min_score"),
                avg_score: row.try_get("avg_score").ok(),
                max_score: row.try_get("max_score").ok(),
                min_rank: row.try_get("min_rank").ok(),
                source_label: row.get("source_label"),
                source_url: row.try_get("source_url").ok(),
            })
            .collect())
    }

    pub async fn list_province_admission_majors_from_scores(
        &self,
        province: &str,
        subject_type: Option<&str>,
        year: Option<i32>,
        limit: i64,
    ) -> Result<Vec<ProvinceAdmissionMajor>> {
        let province_value = province.trim();
        if let Some(year) = year {
            let rows = sqlx::query(
                r#"
                SELECT DISTINCT ON (m.name, s.subject_type, s.batch)
                  s.year,
                  m.name AS major_name,
                  s.subject_type,
                  s.batch,
                  s.admitted_count,
                  s.min_score,
                  s.source_label
                FROM admission_scores s
                JOIN majors m ON m.id = s.major_id
                JOIN provinces p ON p.id = s.province_id
                WHERE (p.code = $1 OR p.name = $1)
                  AND ($2::text IS NULL OR s.subject_type = $2 OR s.subject_type = '未区分')
                  AND s.year = $3
                  AND s.batch NOT ILIKE '%专升本%'
                  AND s.batch NOT ILIKE '%单招%'
                  AND s.batch NOT ILIKE '%预科%'
                ORDER BY m.name, s.subject_type, s.batch, s.min_score
                LIMIT $4
                "#,
            )
            .bind(province_value)
            .bind(subject_type)
            .bind(year)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;

            return Ok(rows
                .into_iter()
                .map(|row| ProvinceAdmissionMajor {
                    year: row.get("year"),
                    major_name: row.get("major_name"),
                    subject_type: row.get("subject_type"),
                    batch: row.get("batch"),
                    admitted_count: row.try_get("admitted_count").ok(),
                    min_score: row.try_get("min_score").ok(),
                    source_label: row.get("source_label"),
                })
                .collect());
        }

        let rows = sqlx::query(
            r#"
            WITH filtered AS (
              SELECT
                latest_year AS year,
                major_name,
                subject_type,
                batch,
                latest_admitted_count AS admitted_count,
                latest_min_score AS min_score,
                source_label
              FROM admission_major_province_coverage
              WHERE (province_code = $1 OR province_name = $1)
                AND ($2::text IS NULL OR subject_type = $2 OR subject_type = '未区分')
                AND batch NOT ILIKE '%专升本%'
                AND batch NOT ILIKE '%单招%'
                AND batch NOT ILIKE '%预科%'
            ),
            target_year AS (
              SELECT MAX(year) AS year FROM filtered
            )
            SELECT DISTINCT ON (major_name, subject_type, batch)
              year, major_name, subject_type, batch, admitted_count, min_score, source_label
            FROM filtered
            WHERE year = (SELECT year FROM target_year)
            ORDER BY major_name, subject_type, batch, min_score
            LIMIT $3
            "#,
        )
        .bind(province_value)
        .bind(subject_type)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| ProvinceAdmissionMajor {
                year: row.get("year"),
                major_name: row.get("major_name"),
                subject_type: row.get("subject_type"),
                batch: row.get("batch"),
                admitted_count: row.try_get("admitted_count").ok(),
                min_score: row.try_get("min_score").ok(),
                source_label: row.get("source_label"),
            })
            .collect())
    }

    pub async fn list_major_admission_provinces_from_scores(
        &self,
        major_slug: &str,
        subject_type: Option<&str>,
        year: Option<i32>,
        limit: i64,
    ) -> Result<Vec<MajorAdmissionProvince>> {
        if let Some(year) = year {
            let rows = sqlx::query(
                r#"
                SELECT DISTINCT ON (p.name, s.subject_type, s.batch)
                  s.year,
                  p.name AS province_name,
                  s.subject_type,
                  s.batch,
                  s.admitted_count,
                  s.min_score,
                  s.source_label
                FROM admission_scores s
                JOIN majors m ON m.id = s.major_id
                JOIN provinces p ON p.id = s.province_id
                WHERE m.slug = $1
                  AND ($2::text IS NULL OR s.subject_type = $2 OR s.subject_type = '未区分')
                  AND s.year = $3
                  AND s.batch NOT ILIKE '%专升本%'
                  AND s.batch NOT ILIKE '%单招%'
                  AND s.batch NOT ILIKE '%预科%'
                ORDER BY p.name, s.subject_type, s.batch, s.min_score
                LIMIT $4
                "#,
            )
            .bind(major_slug)
            .bind(subject_type)
            .bind(year)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;

            return Ok(rows
                .into_iter()
                .map(|row| MajorAdmissionProvince {
                    year: row.get("year"),
                    province_name: row.get("province_name"),
                    subject_type: row.get("subject_type"),
                    batch: row.get("batch"),
                    admitted_count: row.try_get("admitted_count").ok(),
                    min_score: row.try_get("min_score").ok(),
                    source_label: row.get("source_label"),
                })
                .collect());
        }

        let rows = sqlx::query(
            r#"
            WITH filtered AS (
              SELECT
                latest_year AS year,
                province_name,
                subject_type,
                batch,
                latest_admitted_count AS admitted_count,
                latest_min_score AS min_score,
                source_label
              FROM admission_major_province_coverage
              WHERE major_slug = $1
                AND ($2::text IS NULL OR subject_type = $2 OR subject_type = '未区分')
                AND batch NOT ILIKE '%专升本%'
                AND batch NOT ILIKE '%单招%'
                AND batch NOT ILIKE '%预科%'
            ),
            target_year AS (
              SELECT MAX(year) AS year FROM filtered
            )
            SELECT DISTINCT ON (province_name, subject_type, batch)
              year, province_name, subject_type, batch, admitted_count, min_score, source_label
            FROM filtered
            WHERE year = (SELECT year FROM target_year)
            ORDER BY province_name, subject_type, batch, min_score
            LIMIT $3
            "#,
        )
        .bind(major_slug)
        .bind(subject_type)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| MajorAdmissionProvince {
                year: row.get("year"),
                province_name: row.get("province_name"),
                subject_type: row.get("subject_type"),
                batch: row.get("batch"),
                admitted_count: row.try_get("admitted_count").ok(),
                min_score: row.try_get("min_score").ok(),
                source_label: row.get("source_label"),
            })
            .collect())
    }

    pub async fn search_faq(&self, query: &str, limit: i64) -> Result<Vec<FaqEvidence>> {
        let pattern = format!("%{}%", query.trim());
        let rows = sqlx::query(
            r#"
            SELECT id, question, answer, category, source_label
            FROM faq_knowledge
            WHERE status = 'PUBLISHED'
              AND (question ILIKE $1 OR answer ILIKE $1 OR category ILIKE $1)
            ORDER BY updated_at DESC
            LIMIT $2
            "#,
        )
        .bind(pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| FaqEvidence {
                id: row.get("id"),
                question: row.get("question"),
                answer: row.get("answer"),
                category: row.get("category"),
                source_label: row.get("source_label"),
                similarity: Some(1.0),
            })
            .collect())
    }

    pub async fn search_faq_vector(
        &self,
        embedding: &[f32],
        min_similarity: f64,
        limit: i64,
    ) -> Result<Vec<FaqEvidence>> {
        let embedding_literal = pgvector_literal(embedding);
        let rows = sqlx::query(
            r#"
            SELECT DISTINCT ON (fk.id)
              fk.id,
              fk.question,
              fk.answer,
              fk.category,
              fk.source_label,
              1 - (kc.embedding <=> $1::vector) as similarity
            FROM knowledge_chunks kc
            JOIN faq_knowledge fk ON fk.id = kc.faq_knowledge_id
            WHERE kc.embedding IS NOT NULL
              AND fk.status = 'PUBLISHED'
              AND 1 - (kc.embedding <=> $1::vector) >= $2
            ORDER BY fk.id, kc.embedding <=> $1::vector
            LIMIT $3
            "#,
        )
        .bind(embedding_literal)
        .bind(min_similarity)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let mut items = rows
            .into_iter()
            .map(|row| FaqEvidence {
                id: row.get("id"),
                question: row.get("question"),
                answer: row.get("answer"),
                category: row.get("category"),
                source_label: row.get("source_label"),
                similarity: row.try_get("similarity").ok(),
            })
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            right
                .similarity
                .partial_cmp(&left.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        items.truncate(limit as usize);
        Ok(items)
    }

    pub async fn search_policies(
        &self,
        query: &str,
        filters: &KnowledgeSearchFilters,
        limit: i64,
    ) -> Result<Vec<PolicyEvidence>> {
        let pattern = format!("%{}%", query.trim());
        let rows = sqlx::query(
            r#"
            SELECT id, title, category, year, source_label, source_url, content_text
            FROM policy_documents
            WHERE status = 'ACTIVE'
              AND ($2::text IS NULL OR category = $2)
              AND ($3::int IS NULL OR year = $3)
              AND (title ILIKE $1 OR content_text ILIKE $1 OR category ILIKE $1)
            ORDER BY year DESC NULLS LAST, updated_at DESC
            LIMIT $4
            "#,
        )
        .bind(pattern)
        .bind(filters.category.as_deref())
        .bind(filters.year)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| PolicyEvidence {
                id: row.get("id"),
                title: row.get("title"),
                category: row.get("category"),
                year: row.try_get("year").ok(),
                source_label: row.get("source_label"),
                source_url: row.try_get("source_url").ok(),
                content_text: row.get("content_text"),
            })
            .collect())
    }

    pub async fn search_knowledge_chunks_keyword(
        &self,
        query: &str,
        filters: &KnowledgeSearchFilters,
        limit: i64,
    ) -> Result<Vec<VectorChunkEvidence>> {
        let pattern = format!("%{}%", query.trim());
        let rows = sqlx::query(
            r#"
            SELECT kc.id, kc.title, kc.content, kc.metadata, pd.category, pd.year
            FROM knowledge_chunks kc
            LEFT JOIN policy_documents pd ON pd.id = kc.policy_document_id
            WHERE kc.data_version = 'official-pdf-knowledge-v2'
              AND ($2::text IS NULL OR pd.category = $2)
              AND ($3::int IS NULL OR pd.year = $3)
              AND ($4::text IS NULL OR kc.metadata->>'documentKind' = $4)
              AND (kc.title ILIKE $1 OR kc.content ILIKE $1 OR kc.metadata::text ILIKE $1)
            ORDER BY
              CASE WHEN kc.title ILIKE $1 THEN 0 ELSE 1 END,
              length(kc.content) DESC
            LIMIT $5
            "#,
        )
        .bind(pattern)
        .bind(filters.category.as_deref())
        .bind(filters.year)
        .bind(filters.document_kind.as_deref())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| VectorChunkEvidence {
                id: row.get("id"),
                title: row.try_get("title").ok(),
                content: row.get("content"),
                category: row.try_get("category").ok(),
                year: row.try_get("year").ok(),
                similarity: None,
                metadata: row.try_get("metadata").unwrap_or(Value::Null),
            })
            .collect())
    }

    pub async fn search_knowledge_chunks_vector(
        &self,
        embedding: &[f32],
        filters: &KnowledgeSearchFilters,
        limit: i64,
    ) -> Result<Vec<VectorChunkEvidence>> {
        let embedding_literal = pgvector_literal(embedding);
        let rows = sqlx::query(
            r#"
            SELECT
              kc.id,
              kc.title,
              kc.content,
              kc.metadata,
              pd.category,
              pd.year,
              1 - (kc.embedding <=> $1::vector) as similarity
            FROM knowledge_chunks kc
            LEFT JOIN policy_documents pd ON pd.id = kc.policy_document_id
            WHERE kc.embedding IS NOT NULL
              AND kc.data_version = 'official-pdf-knowledge-v2'
              AND ($2::text IS NULL OR pd.category = $2)
              AND ($3::int IS NULL OR pd.year = $3)
              AND ($4::text IS NULL OR kc.metadata->>'documentKind' = $4)
            ORDER BY kc.embedding <=> $1::vector
            LIMIT $5
            "#,
        )
        .bind(embedding_literal)
        .bind(filters.category.as_deref())
        .bind(filters.year)
        .bind(filters.document_kind.as_deref())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| VectorChunkEvidence {
                id: row.get("id"),
                title: row.try_get("title").ok(),
                content: row.get("content"),
                category: row.try_get("category").ok(),
                year: row.try_get("year").ok(),
                similarity: row.try_get("similarity").ok(),
                metadata: row.try_get("metadata").unwrap_or(Value::Null),
            })
            .collect())
    }

    pub async fn search_knowledge_chunks_by_major(
        &self,
        major_name: &str,
        topic_keyword: Option<&str>,
        filters: &KnowledgeSearchFilters,
        limit: i64,
    ) -> Result<Vec<VectorChunkEvidence>> {
        let major_pattern = format!("%{}%", major_name.trim());
        let topic_pattern = topic_keyword.map(|keyword| format!("%{}%", keyword.trim()));
        let rows = sqlx::query(
            r#"
            SELECT kc.id, kc.title, kc.content, kc.metadata, pd.category, pd.year
            FROM knowledge_chunks kc
            LEFT JOIN policy_documents pd ON pd.id = kc.policy_document_id
            WHERE kc.data_version = 'official-pdf-knowledge-v2'
              AND ($3::text IS NULL OR pd.category = $3)
              AND ($4::int IS NULL OR pd.year = $4)
              AND ($5::text IS NULL OR kc.metadata->>'documentKind' = $5)
              AND kc.metadata->>'majorName' ILIKE $1
            ORDER BY
              CASE
                WHEN $2::text IS NOT NULL AND kc.content ILIKE $2 THEN 0
                WHEN $2::text IS NOT NULL AND kc.title ILIKE $2 THEN 1
                ELSE 2
              END,
              COALESCE((kc.metadata->>'sequence')::int, 999999),
              length(kc.content) DESC
            LIMIT $6
            "#,
        )
        .bind(major_pattern)
        .bind(topic_pattern.as_deref())
        .bind(filters.category.as_deref())
        .bind(filters.year)
        .bind(filters.document_kind.as_deref())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| VectorChunkEvidence {
                id: row.get("id"),
                title: row.try_get("title").ok(),
                content: row.get("content"),
                category: row.try_get("category").ok(),
                year: row.try_get("year").ok(),
                similarity: Some(0.92),
                metadata: row.try_get("metadata").unwrap_or(Value::Null),
            })
            .collect())
    }

    pub async fn search_training_plan_chunks_by_major(
        &self,
        major_name: &str,
        topic_keyword: Option<&str>,
        limit: i64,
    ) -> Result<Vec<VectorChunkEvidence>> {
        let major_pattern = format!("%{}%", major_name.trim());
        let topic_pattern = topic_keyword.map(|keyword| format!("%{}%", keyword.trim()));
        let rows = sqlx::query(
            r#"
            SELECT kc.id, kc.title, kc.content, kc.metadata, pd.category, pd.year
            FROM knowledge_chunks kc
            LEFT JOIN policy_documents pd ON pd.id = kc.policy_document_id
            WHERE kc.data_version = 'official-pdf-knowledge-v2'
              AND kc.metadata->>'documentKind' = 'training_plan'
              AND kc.metadata->>'majorName' ILIKE $1
            ORDER BY
              CASE
                WHEN $2::text IS NOT NULL AND kc.content ILIKE $2 THEN 0
                WHEN $2::text IS NOT NULL AND kc.title ILIKE $2 THEN 1
                ELSE 2
              END,
              CASE kc.metadata->>'sectionType'
                WHEN 'training_objectives' THEN 0
                WHEN 'graduation_conditions' THEN 1
                WHEN 'credit_structure' THEN 2
                WHEN 'teaching_plan' THEN 3
                WHEN 'practice_teaching' THEN 4
                WHEN 'semester_weeks' THEN 5
                ELSE 9
              END,
              COALESCE((kc.metadata->>'sequence')::int, 999999),
              length(kc.content) DESC
            LIMIT $3
            "#,
        )
        .bind(major_pattern)
        .bind(topic_pattern.as_deref())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| VectorChunkEvidence {
                id: row.get("id"),
                title: row.try_get("title").ok(),
                content: row.get("content"),
                category: row.try_get("category").ok(),
                year: row.try_get("year").ok(),
                similarity: Some(0.95),
                metadata: row.try_get("metadata").unwrap_or(Value::Null),
            })
            .collect())
    }

    pub async fn list_college_training_plan_majors(
        &self,
        college_name: &str,
    ) -> Result<Vec<CollegeMajorRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT
              kc.metadata->>'college' as college_name,
              kc.metadata->>'majorName' as major_name,
              MIN(kc.title) as title,
              COUNT(*)::bigint as chunk_count,
              MIN(NULLIF(kc.metadata->>'sequence', '')::int) as first_sequence
            FROM knowledge_chunks kc
            LEFT JOIN policy_documents pd ON pd.id = kc.policy_document_id
            WHERE kc.data_version = 'official-pdf-knowledge-v2'
              AND pd.category = '培养方案'
              AND kc.metadata->>'documentKind' = 'training_plan'
              AND kc.metadata->>'college' = $1
              AND COALESCE(kc.metadata->>'majorName', '') <> ''
            GROUP BY kc.metadata->>'college', kc.metadata->>'majorName'
            ORDER BY first_sequence NULLS LAST, major_name
            "#,
        )
        .bind(college_name.trim())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| CollegeMajorRecord {
                college_name: row.get("college_name"),
                major_name: row.get("major_name"),
                title: row.try_get("title").ok(),
                chunk_count: row.get::<i64, _>("chunk_count"),
            })
            .collect())
    }

    pub async fn get_or_create_conversation(
        &self,
        conversation_id: Option<&str>,
    ) -> Result<String> {
        let id = conversation_id
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_owned())
            .unwrap_or_else(|| format!("rust_chat_{}", uuid::Uuid::new_v4()));

        sqlx::query(
            r#"
            INSERT INTO conversations (id, session_key, intended_majors, updated_at)
            VALUES ($1, $1, '[]'::jsonb, now())
            ON CONFLICT (session_key) DO UPDATE SET updated_at = now()
            "#,
        )
        .bind(&id)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    pub async fn get_conversation_history(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationHistory>> {
        self.get_conversation_history_with_limit(conversation_id, None)
            .await
    }

    pub async fn get_conversation_recent_history(
        &self,
        conversation_id: &str,
        limit: i64,
    ) -> Result<Option<ConversationHistory>> {
        self.get_conversation_history_with_limit(conversation_id, Some(limit.max(1)))
            .await
    }

    async fn get_conversation_history_with_limit(
        &self,
        conversation_id: &str,
        limit: Option<i64>,
    ) -> Result<Option<ConversationHistory>> {
        let conversation = sqlx::query(
            r#"
            SELECT id, session_key
            FROM conversations
            WHERE id = $1 OR session_key = $1
            LIMIT 1
            "#,
        )
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some(conversation) = conversation else {
            return Ok(None);
        };

        let id: String = conversation.get("id");
        let rows = if let Some(limit) = limit {
            sqlx::query(
                r#"
                SELECT role, content, structured_payload, citations, created_at
                FROM (
                  SELECT role, content, structured_payload, citations, created_at
                  FROM conversation_messages
                  WHERE conversation_id = $1
                  ORDER BY created_at DESC
                  LIMIT $2
                ) recent_messages
                ORDER BY created_at ASC
                "#,
            )
            .bind(&id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT role, content, structured_payload, citations, created_at
                FROM conversation_messages
                WHERE conversation_id = $1
                ORDER BY created_at ASC
                "#,
            )
            .bind(&id)
            .fetch_all(&self.pool)
            .await?
        };

        let messages = rows
            .into_iter()
            .map(|row| {
                let structured_payload = row
                    .try_get::<Value, _>("structured_payload")
                    .ok()
                    .and_then(|value| serde_json::from_value::<ChatStructuredResult>(value).ok());
                let citations = row
                    .try_get::<Value, _>("citations")
                    .ok()
                    .and_then(|value| serde_json::from_value::<Vec<ChatCitation>>(value).ok())
                    .unwrap_or_default();
                ConversationMessage {
                    role: row.get("role"),
                    content: row.get("content"),
                    structured_payload,
                    citations,
                    created_at: row.try_get::<DateTime<Utc>, _>("created_at").ok(),
                }
            })
            .collect();

        Ok(Some(ConversationHistory {
            id,
            session_key: conversation.get("session_key"),
            messages,
        }))
    }

    pub async fn append_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        structured_payload: Option<&ChatStructuredResult>,
        citations: &[ChatCitation],
    ) -> Result<()> {
        let structured_value = structured_payload.map(serde_json::to_value).transpose()?;
        let citations_value = serde_json::to_value(citations)?;
        sqlx::query(
            r#"
            INSERT INTO conversation_messages
              (id, conversation_id, role, content, structured_payload, citations)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(format!("rust_msg_{}", uuid::Uuid::new_v4()))
        .bind(conversation_id)
        .bind(role)
        .bind(content)
        .bind(structured_value)
        .bind(citations_value)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
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

fn pgvector_literal(values: &[f32]) -> String {
    let body = values
        .iter()
        .map(|value| {
            if value.is_finite() {
                value.to_string()
            } else {
                "0".to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{body}]")
}

pub fn summarize_score_records(records: &[AdmissionScoreRecord]) -> ScoreSummary {
    let mut years = records.iter().map(|record| record.year).collect::<Vec<_>>();
    years.sort_unstable();
    years.dedup();
    years.reverse();

    let mut source_labels = records
        .iter()
        .map(|record| record.source_label.clone())
        .collect::<Vec<_>>();
    source_labels.sort();
    source_labels.dedup();

    ScoreSummary {
        total_records: records.len(),
        years,
        source_labels,
    }
}

pub fn memory_from_profile(profile: Option<&domain::ChatProfile>) -> ResolvedMemory {
    let Some(profile) = profile else {
        return ResolvedMemory::default();
    };
    ResolvedMemory {
        province_name: profile.province.clone(),
        subject_type: profile.subject_type.clone(),
        score: profile.score,
        rank: profile.rank,
        ..ResolvedMemory::default()
    }
}
