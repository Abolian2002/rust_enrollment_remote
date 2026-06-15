use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use domain::{
    AdminAdmissionsAnalyticsSnapshot, AdminAuditLogItem, AdminAuditLogList, AdminBehaviorCard,
    AdminBigScreenSnapshot, AdminChartDatum, AdminConversationDetail, AdminConversationList,
    AdminConversationListItem, AdminDashboardSnapshot, AdminEvaluationList, AdminEvaluationListItem,
    AdminEvaluationSummarySnapshot, AdminFaqItem, AdminFaqList, AdminFeedbackItem,
    AdminInsightsSnapshot, AdminKnowledgeChunkItem, AdminKnowledgeChunkList,
    AdminKnowledgeCoverageSnapshot, AdminRealtimeMessage, AdminSettings, AdminSpecialSnapshot,
    AdminStat, AdminTicketItem, AdminTicketList, AdminTopQuestion, AdmissionScoreRecord,
    ChatCitation, ChatStructuredResult, ConversationHistory, ConversationMessage, FaqEvidence,
    LatestScore, MajorAdmissionProvince, PolicyEvidence, ProvinceAdmissionMajor, ResolvedMemory,
    ScoreSummary, VectorChunkEvidence,
};
use serde_json::Value;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::time::Duration;
use uuid::Uuid;

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

    pub async fn admin_dashboard_snapshot(&self) -> Result<AdminDashboardSnapshot> {
        let counts = sqlx::query(
            r#"
            SELECT
              (SELECT COUNT(*)::bigint FROM conversations) AS conversation_count,
              (SELECT COUNT(*)::bigint FROM conversation_messages WHERE role = 'user') AS user_message_count,
              (SELECT COUNT(*)::bigint FROM conversation_messages WHERE role = 'assistant') AS assistant_message_count,
              (SELECT COUNT(*)::bigint FROM faq_knowledge WHERE status = 'PUBLISHED') AS faq_count,
              (SELECT COUNT(*)::bigint FROM knowledge_chunks WHERE data_version = 'official-pdf-knowledge-v2') AS chunk_count,
              (SELECT COUNT(DISTINCT province_id)::bigint FROM admission_scores) AS province_count,
              to_char(now(), 'YYYY-MM-DD HH24:MI') AS updated_at
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        let conversation_count = counts.get::<i64, _>("conversation_count");
        let user_message_count = counts.get::<i64, _>("user_message_count");
        let assistant_message_count = counts.get::<i64, _>("assistant_message_count");
        let faq_count = counts.get::<i64, _>("faq_count");
        let chunk_count = counts.get::<i64, _>("chunk_count");
        let province_count = counts.get::<i64, _>("province_count");
        let avg_questions = if conversation_count > 0 {
            format!(
                "{:.1}",
                user_message_count as f64 / conversation_count as f64
            )
        } else {
            "0.0".to_owned()
        };

        let trend_rows = sqlx::query(
            r#"
            SELECT
              to_char(day::date, 'MM-DD') AS label,
              COALESCE(COUNT(m.id), 0)::bigint AS value
            FROM generate_series(current_date - interval '13 days', current_date, interval '1 day') AS day
            LEFT JOIN conversation_messages m
              ON m.role = 'user'
             AND m.created_at >= day
             AND m.created_at < day + interval '1 day'
            GROUP BY day
            ORDER BY day
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let trend_days = trend_rows
            .iter()
            .map(|row| row.get::<String, _>("label"))
            .collect::<Vec<_>>();
        let trend_values = trend_rows
            .iter()
            .map(|row| row.get::<i64, _>("value"))
            .collect::<Vec<_>>();

        let hourly_rows = sqlx::query(
            r#"
            SELECT
              hour,
              COALESCE(COUNT(m.id), 0)::bigint AS value
            FROM generate_series(0, 23) AS hour
            LEFT JOIN conversation_messages m
              ON m.role = 'user'
             AND EXTRACT(hour FROM m.created_at)::int = hour
             AND m.created_at >= now() - interval '30 days'
            GROUP BY hour
            ORDER BY hour
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let hourly_values = hourly_rows
            .into_iter()
            .map(|row| row.get::<i64, _>("value"))
            .collect::<Vec<_>>();

        let hot_rows = sqlx::query(
            r#"
            SELECT
              left(regexp_replace(trim(content), '\s+', ' ', 'g'), 80) AS question,
              COUNT(*)::bigint AS count
            FROM conversation_messages
            WHERE role = 'user'
              AND trim(content) <> ''
            GROUP BY left(regexp_replace(trim(content), '\s+', ' ', 'g'), 80)
            ORDER BY count DESC, max(created_at) DESC
            LIMIT 20
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let hot_questions = hot_rows
            .into_iter()
            .map(|row| {
                (
                    row.get::<String, _>("question"),
                    format!("{}次", row.get::<i64, _>("count")),
                )
            })
            .collect::<Vec<_>>();

        let category_rows = sqlx::query(
            r#"
            SELECT category, COUNT(*)::bigint AS count
            FROM (
              SELECT CASE
                WHEN content ILIKE '%分数%' OR content ILIKE '%录取线%' OR content ILIKE '%位次%' OR content ILIKE '%多少分%' THEN '分数位次'
                WHEN content ILIKE '%调剂%' OR content ILIKE '%同分%' OR content ILIKE '%录取规则%' OR content ILIKE '%投档%' THEN '录取规则'
                WHEN content ILIKE '%专业%' OR content ILIKE '%课程%' OR content ILIKE '%培养方案%' OR content ILIKE '%学院%' THEN '专业介绍'
                WHEN content ILIKE '%公费师范%' OR content ILIKE '%免费师范%' THEN '公费师范'
                WHEN content ILIKE '%优师%' THEN '优师专项'
                ELSE '其他'
              END AS category
              FROM conversation_messages
              WHERE role = 'user'
            ) categorized
            GROUP BY category
            ORDER BY count DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let category_stats = category_rows
            .into_iter()
            .map(|row| AdminChartDatum {
                name: row.get("category"),
                value: row.get("count"),
            })
            .collect::<Vec<_>>();

        let province_rows = sqlx::query(
            r#"
            SELECT
              COALESCE(p.name, NULLIF(c.province_code, ''), '未知') AS province,
              COUNT(*)::bigint AS count
            FROM conversations c
            LEFT JOIN provinces p ON p.code = c.province_code OR p.name = c.province_code
            GROUP BY COALESCE(p.name, NULLIF(c.province_code, ''), '未知')
            ORDER BY count DESC, province
            LIMIT 12
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let province_bars = province_rows
            .into_iter()
            .map(|row| (row.get::<String, _>("province"), row.get::<i64, _>("count")))
            .collect::<Vec<_>>();

        Ok(AdminDashboardSnapshot {
            updated_at: counts.get("updated_at"),
            stats: vec![
                AdminStat {
                    label: "会话总数".to_owned(),
                    value: format_number(conversation_count),
                    delta: None,
                    tone: Some("blue".to_owned()),
                },
                AdminStat {
                    label: "咨询用户数".to_owned(),
                    value: format_number(conversation_count),
                    delta: None,
                    tone: Some("green".to_owned()),
                },
                AdminStat {
                    label: "用户提问数".to_owned(),
                    value: format_number(user_message_count),
                    delta: None,
                    tone: Some("cyan".to_owned()),
                },
                AdminStat {
                    label: "智能回答数".to_owned(),
                    value: format_number(assistant_message_count),
                    delta: None,
                    tone: Some("purple".to_owned()),
                },
                AdminStat {
                    label: "人均提问次数".to_owned(),
                    value: avg_questions,
                    delta: None,
                    tone: Some("amber".to_owned()),
                },
                AdminStat {
                    label: "FAQ/文档片段".to_owned(),
                    value: format!(
                        "{}/{}",
                        format_number(faq_count),
                        format_number(chunk_count)
                    ),
                    delta: Some(format!("覆盖{}省", province_count)),
                    tone: Some("blue".to_owned()),
                },
            ],
            trend_days,
            trend_values,
            hourly_values,
            hot_questions,
            category_stats,
            province_bars,
        })
    }

    pub async fn admin_insights_snapshot(&self) -> Result<AdminInsightsSnapshot> {
        let counts = sqlx::query(
            r#"
            SELECT
              (SELECT COUNT(DISTINCT conversation_id)::bigint FROM conversation_messages WHERE role = 'user' AND created_at >= now() - interval '30 days') AS active_users,
              (SELECT COUNT(*)::bigint FROM conversation_messages WHERE role = 'user' AND created_at >= now() - interval '30 days') AS question_count,
              (SELECT COUNT(DISTINCT c.province_code)::bigint FROM conversations c WHERE NULLIF(c.province_code, '') IS NOT NULL) AS province_count,
              (SELECT COUNT(*)::bigint FROM conversation_messages WHERE role = 'user') AS total_questions,
              to_char(now(), 'YYYY-MM-DD HH24:MI') AS updated_at
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        let active_users = counts.get::<i64, _>("active_users");
        let question_count = counts.get::<i64, _>("question_count");
        let province_count = counts.get::<i64, _>("province_count");
        let total_questions = counts.get::<i64, _>("total_questions").max(1);

        let category_stats = self.admin_question_category_stats().await?;
        let province_bars = self.admin_province_bars(12).await?;
        let top_questions = self.admin_top_questions(20, total_questions).await?;

        let word_rows = sqlx::query(
            r#"
            WITH keywords(word) AS (
              VALUES
                ('录取分数'), ('位次'), ('概率'), ('招生简章'), ('培养方案'), ('专业'),
                ('课程'), ('公费师范'), ('优师计划'), ('调剂'), ('同分'), ('体检'),
                ('宿舍'), ('学费'), ('就业'), ('考研'), ('校区'), ('招生电话')
            )
            SELECT word, COUNT(m.id)::bigint AS count
            FROM keywords
            LEFT JOIN conversation_messages m
              ON m.role = 'user' AND m.content ILIKE '%' || word || '%'
            GROUP BY word
            HAVING COUNT(m.id) > 0
            ORDER BY count DESC, word
            LIMIT 24
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let word_cloud = word_rows
            .into_iter()
            .map(|row| AdminChartDatum {
                name: row.get("word"),
                value: row.get("count"),
            })
            .collect::<Vec<_>>();

        Ok(AdminInsightsSnapshot {
            updated_at: counts.get("updated_at"),
            stats: vec![
                AdminStat {
                    label: "近30天咨询会话".to_owned(),
                    value: format_number(active_users),
                    delta: None,
                    tone: Some("blue".to_owned()),
                },
                AdminStat {
                    label: "近30天用户提问".to_owned(),
                    value: format_number(question_count),
                    delta: None,
                    tone: Some("green".to_owned()),
                },
                AdminStat {
                    label: "覆盖省份".to_owned(),
                    value: format_number(province_count),
                    delta: None,
                    tone: Some("cyan".to_owned()),
                },
            ],
            category_stats,
            province_bars,
            top_questions,
            word_cloud,
            summary: "统计来自真实对话日志。当前后台按分数位次、录取规则、专业介绍、专项政策和校园生活等通用维度归类，用于发现学生和家长近期最关心的问题。".to_owned(),
        })
    }

    pub async fn admin_special_snapshot(&self) -> Result<AdminSpecialSnapshot> {
        // Plan D: run independent queries in parallel with tokio::try_join!
        let (
            total_questions,
            updated_at,
            plan_rows,
            major_attention,
            policy_rows,
            normal_rows,
        ) = tokio::try_join!(
            self.admin_total_user_questions(),
            self.admin_updated_at(),
            self.fetch_special_plan_rows(),
            self.admin_major_attention(10),
            self.fetch_policy_rows(),
            self.fetch_normal_vs_non_normal(),
        )?;
        let total_questions = total_questions.max(1);

        let special_plans = plan_rows
            .iter()
            .map(|row| {
                let count = row.get::<i64, _>("count");
                (
                    row.get::<String, _>("name"),
                    count,
                    format_share(count, total_questions),
                    "真实咨询".to_owned(),
                )
            })
            .collect::<Vec<_>>();

        let normal_vs_non_normal = normal_rows
            .into_iter()
            .map(|row| AdminChartDatum {
                name: row.get("name"),
                value: row.get("count"),
            })
            .collect::<Vec<_>>();

        let policy_stats = policy_rows
            .into_iter()
            .map(|row| (row.get::<String, _>("name"), row.get::<i64, _>("count")))
            .collect::<Vec<_>>();

        let count_for = |name: &str| -> i64 {
            special_plans
                .iter()
                .find(|(item_name, _, _, _)| item_name == name)
                .map(|(_, count, _, _)| *count)
                .unwrap_or(0)
        };

        Ok(AdminSpecialSnapshot {
            updated_at,
            stats: vec![
                AdminStat {
                    label: "公费师范咨询量".to_owned(),
                    value: format_number(count_for("公费师范生")),
                    delta: None,
                    tone: Some("blue".to_owned()),
                },
                AdminStat {
                    label: "优师计划咨询量".to_owned(),
                    value: format_number(count_for("优师计划")),
                    delta: None,
                    tone: Some("green".to_owned()),
                },
                AdminStat {
                    label: "振兴龙江专项咨询量".to_owned(),
                    value: format_number(count_for("振兴龙江计划")),
                    delta: None,
                    tone: Some("amber".to_owned()),
                },
            ],
            normal_vs_non_normal,
            special_plans,
            major_attention,
            policy_stats,
        })
    }

    pub async fn admin_admissions_analytics_snapshot(
        &self,
    ) -> Result<AdminAdmissionsAnalyticsSnapshot> {
        let counts = sqlx::query(
            r#"
            SELECT
              COUNT(*)::bigint AS score_count,
              COUNT(DISTINCT province_id)::bigint AS province_count,
              COUNT(DISTINCT major_id)::bigint AS major_count,
              MIN(year) AS min_year,
              MAX(year) AS max_year,
              to_char(now(), 'YYYY-MM-DD HH24:MI') AS updated_at
            FROM admission_scores
            WHERE batch NOT ILIKE '%专升本%'
              AND batch NOT ILIKE '%单招%'
              AND batch NOT ILIKE '%预科%'
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        let year_rows = sqlx::query(
            r#"
            SELECT year::text AS name, COUNT(*)::bigint AS count
            FROM admission_scores
            WHERE batch NOT ILIKE '%专升本%'
              AND batch NOT ILIKE '%单招%'
              AND batch NOT ILIKE '%预科%'
            GROUP BY year
            ORDER BY year
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let year_counts = year_rows
            .into_iter()
            .map(|row| AdminChartDatum {
                name: row.get("name"),
                value: row.get("count"),
            })
            .collect::<Vec<_>>();

        let province_rows = sqlx::query(
            r#"
            SELECT p.name, COUNT(DISTINCT s.major_id)::bigint AS count
            FROM admission_scores s
            JOIN provinces p ON p.id = s.province_id
            WHERE s.batch NOT ILIKE '%专升本%'
              AND s.batch NOT ILIKE '%单招%'
              AND s.batch NOT ILIKE '%预科%'
            GROUP BY p.name
            ORDER BY count DESC, p.name
            LIMIT 20
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let province_coverage = province_rows
            .into_iter()
            .map(|row| (row.get::<String, _>("name"), row.get::<i64, _>("count")))
            .collect::<Vec<_>>();

        let subject_rows = sqlx::query(
            r#"
            SELECT subject_type AS name, COUNT(*)::bigint AS count
            FROM admission_scores
            WHERE batch NOT ILIKE '%专升本%'
              AND batch NOT ILIKE '%单招%'
              AND batch NOT ILIKE '%预科%'
            GROUP BY subject_type
            ORDER BY count DESC, subject_type
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let subject_distribution = subject_rows
            .into_iter()
            .map(|row| AdminChartDatum {
                name: row.get("name"),
                value: row.get("count"),
            })
            .collect::<Vec<_>>();

        let major_rows = sqlx::query(
            r#"
            SELECT m.name, COUNT(DISTINCT s.province_id)::bigint AS count
            FROM admission_scores s
            JOIN majors m ON m.id = s.major_id
            WHERE s.batch NOT ILIKE '%专升本%'
              AND s.batch NOT ILIKE '%单招%'
              AND s.batch NOT ILIKE '%预科%'
            GROUP BY m.name
            ORDER BY count DESC, m.name
            LIMIT 15
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let top_majors = major_rows
            .into_iter()
            .map(|row| (row.get::<String, _>("name"), row.get::<i64, _>("count")))
            .collect::<Vec<_>>();

        let min_year = counts.try_get::<Option<i32>, _>("min_year").ok().flatten();
        let max_year = counts.try_get::<Option<i32>, _>("max_year").ok().flatten();
        let year_label = match min_year.zip(max_year) {
            Some((min_year, max_year)) if min_year != max_year => format!("{min_year}-{max_year}"),
            Some((year, _)) => year.to_string(),
            None => "暂无".to_owned(),
        };

        Ok(AdminAdmissionsAnalyticsSnapshot {
            updated_at: counts.get("updated_at"),
            stats: vec![
                AdminStat {
                    label: "录取统计记录".to_owned(),
                    value: format_number(counts.get("score_count")),
                    delta: Some(year_label),
                    tone: Some("blue".to_owned()),
                },
                AdminStat {
                    label: "覆盖省份".to_owned(),
                    value: format_number(counts.get("province_count")),
                    delta: None,
                    tone: Some("green".to_owned()),
                },
                AdminStat {
                    label: "覆盖专业".to_owned(),
                    value: format_number(counts.get("major_count")),
                    delta: None,
                    tone: Some("cyan".to_owned()),
                },
            ],
            year_counts,
            province_coverage,
            subject_distribution,
            top_majors,
        })
    }

    pub async fn admin_knowledge_coverage_snapshot(
        &self,
    ) -> Result<AdminKnowledgeCoverageSnapshot> {
        let counts = sqlx::query(
            r#"
            SELECT
              (SELECT COUNT(*)::bigint FROM faq_knowledge WHERE status = 'PUBLISHED') AS faq_count,
              (SELECT COUNT(*)::bigint FROM policy_documents WHERE status::text = 'ACTIVE') AS policy_count,
              (SELECT COUNT(*)::bigint FROM knowledge_chunks WHERE data_version = 'official-pdf-knowledge-v2') AS chunk_count,
              (SELECT COUNT(DISTINCT metadata->>'college')::bigint FROM knowledge_chunks WHERE data_version = 'official-pdf-knowledge-v2' AND NULLIF(metadata->>'college', '') IS NOT NULL) AS college_count,
              to_char(now(), 'YYYY-MM-DD HH24:MI') AS updated_at
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        let kind_rows = sqlx::query(
            r#"
            SELECT COALESCE(NULLIF(metadata->>'documentKind', ''), source_type::text) AS name, COUNT(*)::bigint AS count
            FROM knowledge_chunks
            WHERE data_version = 'official-pdf-knowledge-v2'
            GROUP BY COALESCE(NULLIF(metadata->>'documentKind', ''), source_type::text)
            ORDER BY count DESC, name
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let document_kinds = kind_rows
            .into_iter()
            .map(|row| AdminChartDatum {
                name: row.get("name"),
                value: row.get("count"),
            })
            .collect::<Vec<_>>();

        let college_rows = sqlx::query(
            r#"
            SELECT metadata->>'college' AS name, COUNT(*)::bigint AS count
            FROM knowledge_chunks
            WHERE data_version = 'official-pdf-knowledge-v2'
              AND NULLIF(metadata->>'college', '') IS NOT NULL
            GROUP BY metadata->>'college'
            ORDER BY count DESC, name
            LIMIT 20
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let college_chunks = college_rows
            .into_iter()
            .map(|row| (row.get::<String, _>("name"), row.get::<i64, _>("count")))
            .collect::<Vec<_>>();

        let faq_rows = sqlx::query(
            r#"
            SELECT category AS name, COUNT(*)::bigint AS count
            FROM faq_knowledge
            WHERE status = 'PUBLISHED'
            GROUP BY category
            ORDER BY count DESC, category
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let faq_categories = faq_rows
            .into_iter()
            .map(|row| AdminChartDatum {
                name: row.get("name"),
                value: row.get("count"),
            })
            .collect::<Vec<_>>();

        let policy_rows = sqlx::query(
            r#"
            SELECT COALESCE(year::text, '未标注') AS name, COUNT(*)::bigint AS count
            FROM policy_documents
            WHERE status::text = 'ACTIVE'
            GROUP BY COALESCE(year::text, '未标注')
            ORDER BY name DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let policy_years = policy_rows
            .into_iter()
            .map(|row| AdminChartDatum {
                name: row.get("name"),
                value: row.get("count"),
            })
            .collect::<Vec<_>>();

        Ok(AdminKnowledgeCoverageSnapshot {
            updated_at: counts.get("updated_at"),
            stats: vec![
                AdminStat {
                    label: "固定 FAQ".to_owned(),
                    value: format_number(counts.get("faq_count")),
                    delta: None,
                    tone: Some("green".to_owned()),
                },
                AdminStat {
                    label: "政策文档".to_owned(),
                    value: format_number(counts.get("policy_count")),
                    delta: None,
                    tone: Some("blue".to_owned()),
                },
                AdminStat {
                    label: "PDF/FAQ 向量片段".to_owned(),
                    value: format_number(counts.get("chunk_count")),
                    delta: None,
                    tone: Some("cyan".to_owned()),
                },
                AdminStat {
                    label: "培养方案覆盖学院".to_owned(),
                    value: format_number(counts.get("college_count")),
                    delta: None,
                    tone: Some("purple".to_owned()),
                },
            ],
            document_kinds,
            college_chunks,
            faq_categories,
            policy_years,
        })
    }

    pub async fn admin_big_screen_snapshot(&self) -> Result<AdminBigScreenSnapshot> {
        let updated_at = self.admin_updated_at().await?;
        let total_questions = self.admin_total_user_questions().await?.max(1);
        let today_questions = self
            .admin_user_question_count("current_date", "now()")
            .await?;
        let today_users = self
            .admin_distinct_conversation_count("current_date", "now()")
            .await?;
        let week_questions = self
            .admin_user_question_count("current_date - interval '6 days'", "now()")
            .await?;
        let week_users = self
            .admin_distinct_conversation_count("current_date - interval '6 days'", "now()")
            .await?;
        let faq_count = sqlx::query(
            "SELECT COUNT(*)::bigint AS count FROM faq_knowledge WHERE status = 'PUBLISHED'",
        )
        .fetch_one(&self.pool)
        .await?
        .get::<i64, _>("count");
        let chunk_count = sqlx::query("SELECT COUNT(*)::bigint AS count FROM knowledge_chunks WHERE data_version = 'official-pdf-knowledge-v2'")
            .fetch_one(&self.pool)
            .await?
            .get::<i64, _>("count");

        let map_data = self
            .admin_province_bars(34)
            .await?
            .into_iter()
            .map(|(name, value)| AdminChartDatum { name, value })
            .collect::<Vec<_>>();

        let realtime_rows = sqlx::query(
            r#"
            SELECT
              COALESCE(p.name, NULLIF(c.province_code, ''), '未知') AS province,
              left(regexp_replace(trim(m.content), '\s+', ' ', 'g'), 70) AS question,
              COALESCE((
                SELECT left(regexp_replace(trim(a.content), '\s+', ' ', 'g'), 80)
                FROM conversation_messages a
                WHERE a.conversation_id = m.conversation_id
                  AND a.role = 'assistant'
                  AND a.created_at >= m.created_at
                ORDER BY a.created_at ASC
                LIMIT 1
              ), '已回复') AS answer,
              to_char(m.created_at, 'HH24:MI') AS time
            FROM conversation_messages m
            JOIN conversations c ON c.id = m.conversation_id
            LEFT JOIN provinces p ON p.code = c.province_code OR p.name = c.province_code
            WHERE m.role = 'user'
              AND trim(m.content) <> ''
            ORDER BY m.created_at DESC
            LIMIT 12
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let realtime_messages = realtime_rows
            .into_iter()
            .map(|row| AdminRealtimeMessage {
                province: row.get("province"),
                question: row.get("question"),
                answer: row.get("answer"),
                time: row.get("time"),
            })
            .collect::<Vec<_>>();

        let day_points = self.admin_recent_user_question_points(7).await?;
        let top_questions = self.admin_top_questions(10, total_questions).await?;

        Ok(AdminBigScreenSnapshot {
            updated_at,
            big_stats: vec![
                AdminStat {
                    label: "今日咨询用户".to_owned(),
                    value: format_number(today_users),
                    delta: None,
                    tone: Some("blue".to_owned()),
                },
                AdminStat {
                    label: "今日咨询问答".to_owned(),
                    value: format_number(today_questions),
                    delta: None,
                    tone: Some("green".to_owned()),
                },
                AdminStat {
                    label: "近7天用户".to_owned(),
                    value: format_number(week_users),
                    delta: None,
                    tone: Some("cyan".to_owned()),
                },
                AdminStat {
                    label: "近7天问答".to_owned(),
                    value: format_number(week_questions),
                    delta: None,
                    tone: Some("purple".to_owned()),
                },
                AdminStat {
                    label: "FAQ".to_owned(),
                    value: format_number(faq_count),
                    delta: None,
                    tone: Some("amber".to_owned()),
                },
                AdminStat {
                    label: "文档片段".to_owned(),
                    value: format_number(chunk_count),
                    delta: None,
                    tone: Some("green".to_owned()),
                },
            ],
            map_data,
            realtime_messages,
            top_questions,
            behavior_cards: vec![
                AdminBehaviorCard {
                    label: "今日咨询用户".to_owned(),
                    value: format_number(today_users),
                    delta: "+0.0%".to_owned(),
                    points: day_points.clone(),
                },
                AdminBehaviorCard {
                    label: "今日咨询问答".to_owned(),
                    value: format_number(today_questions),
                    delta: "+0.0%".to_owned(),
                    points: day_points.clone(),
                },
                AdminBehaviorCard {
                    label: "近7天用户".to_owned(),
                    value: format_number(week_users),
                    delta: "+0.0%".to_owned(),
                    points: day_points.clone(),
                },
                AdminBehaviorCard {
                    label: "近7天问答".to_owned(),
                    value: format_number(week_questions),
                    delta: "+0.0%".to_owned(),
                    points: day_points,
                },
            ],
            insight: "大屏数据来自真实咨询日志、FAQ 和文档知识库。当前咨询主要围绕录取分数、专业选择、录取政策、专项计划和校园生活展开。".to_owned(),
        })
    }

    async fn admin_updated_at(&self) -> Result<String> {
        Ok(
            sqlx::query("SELECT to_char(now(), 'YYYY-MM-DD HH24:MI') AS updated_at")
                .fetch_one(&self.pool)
                .await?
                .get("updated_at"),
        )
    }

    async fn admin_total_user_questions(&self) -> Result<i64> {
        Ok(sqlx::query(
            "SELECT COUNT(*)::bigint AS count FROM conversation_messages WHERE role = 'user'",
        )
        .fetch_one(&self.pool)
        .await?
        .get("count"))
    }

    // ---- Helper methods extracted for parallel execution (Plan D) ----

    async fn fetch_special_plan_rows(&self) -> Result<Vec<sqlx::postgres::PgRow>> {
        Ok(sqlx::query(
            r#"
            WITH plans(name, pattern) AS (
              VALUES
                ('公费师范生', '%公费师范%'),
                ('优师计划', '%优师%'),
                ('振兴龙江计划', '%振兴龙江%'),
                ('艺术类招生', '%艺术%'),
                ('师范类专业', '%师范%')
            )
            SELECT name, COUNT(m.id)::bigint AS count
            FROM plans
            LEFT JOIN conversation_messages m
              ON m.role = 'user' AND m.content ILIKE pattern
            GROUP BY name
            ORDER BY count DESC, name
            "#,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn fetch_policy_rows(&self) -> Result<Vec<sqlx::postgres::PgRow>> {
        Ok(sqlx::query(
            r#"
            WITH policies(name, pattern) AS (
              VALUES
                ('投档比例', '%投档%'),
                ('调剂退档规则', '%调剂%'),
                ('同分录取规则', '%同分%'),
                ('单科成绩要求', '%单科%'),
                ('体检限制专业', '%体检%'),
                ('加分政策', '%加分%'),
                ('语种要求', '%语种%')
            )
            SELECT name, COUNT(m.id)::bigint AS count
            FROM policies
            LEFT JOIN conversation_messages m
              ON m.role = 'user' AND m.content ILIKE pattern
            GROUP BY name
            ORDER BY count DESC, name
            "#,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn fetch_normal_vs_non_normal(&self) -> Result<Vec<sqlx::postgres::PgRow>> {
        Ok(sqlx::query(
            r#"
            SELECT
              CASE WHEN m.is_normal_major THEN '师范类' ELSE '非师范类' END AS name,
              COUNT(DISTINCT cm.id)::bigint AS count
            FROM majors m
            JOIN conversation_messages cm
              ON cm.role = 'user' AND cm.content ILIKE '%' || m.name || '%'
            GROUP BY m.is_normal_major
            ORDER BY count DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn admin_question_category_stats(&self) -> Result<Vec<AdminChartDatum>> {
        let rows = sqlx::query(
            r#"
            SELECT category, COUNT(*)::bigint AS count
            FROM (
              SELECT CASE
                WHEN content ILIKE '%分数%' OR content ILIKE '%录取线%' OR content ILIKE '%位次%' OR content ILIKE '%概率%' OR content ILIKE '%多少分%' THEN '分数位次'
                WHEN content ILIKE '%调剂%' OR content ILIKE '%同分%' OR content ILIKE '%录取规则%' OR content ILIKE '%投档%' OR content ILIKE '%体检%' OR content ILIKE '%语种%' THEN '录取规则'
                WHEN content ILIKE '%专业%' OR content ILIKE '%课程%' OR content ILIKE '%培养方案%' OR content ILIKE '%学院%' THEN '专业介绍'
                WHEN content ILIKE '%公费师范%' OR content ILIKE '%免费师范%' THEN '公费师范'
                WHEN content ILIKE '%优师%' OR content ILIKE '%振兴龙江%' OR content ILIKE '%专项%' THEN '专项计划'
                WHEN content ILIKE '%宿舍%' OR content ILIKE '%食堂%' OR content ILIKE '%校区%' OR content ILIKE '%学费%' OR content ILIKE '%奖学金%' THEN '校园生活'
                ELSE '其他'
              END AS category
              FROM conversation_messages
              WHERE role = 'user'
            ) categorized
            GROUP BY category
            ORDER BY count DESC, category
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| AdminChartDatum {
                name: row.get("category"),
                value: row.get("count"),
            })
            .collect())
    }

    async fn admin_province_bars(&self, limit: i64) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            r#"
            SELECT
              COALESCE(p.name, NULLIF(c.province_code, ''), '未知') AS province,
              COUNT(*)::bigint AS count
            FROM conversations c
            LEFT JOIN provinces p ON p.code = c.province_code OR p.name = c.province_code
            GROUP BY COALESCE(p.name, NULLIF(c.province_code, ''), '未知')
            ORDER BY count DESC, province
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| (row.get::<String, _>("province"), row.get::<i64, _>("count")))
            .collect())
    }

    async fn admin_top_questions(
        &self,
        limit: i64,
        total_questions: i64,
    ) -> Result<Vec<AdminTopQuestion>> {
        let rows = sqlx::query(
            r#"
            SELECT
              question,
              CASE
                WHEN question ILIKE '%分数%' OR question ILIKE '%录取线%' OR question ILIKE '%位次%' OR question ILIKE '%概率%' OR question ILIKE '%多少分%' THEN '分数位次'
                WHEN question ILIKE '%调剂%' OR question ILIKE '%同分%' OR question ILIKE '%录取规则%' OR question ILIKE '%投档%' OR question ILIKE '%体检%' OR question ILIKE '%语种%' THEN '录取规则'
                WHEN question ILIKE '%专业%' OR question ILIKE '%课程%' OR question ILIKE '%培养方案%' OR question ILIKE '%学院%' THEN '专业介绍'
                WHEN question ILIKE '%公费师范%' OR question ILIKE '%免费师范%' THEN '公费师范'
                WHEN question ILIKE '%优师%' OR question ILIKE '%振兴龙江%' OR question ILIKE '%专项%' THEN '专项计划'
                WHEN question ILIKE '%宿舍%' OR question ILIKE '%食堂%' OR question ILIKE '%校区%' OR question ILIKE '%学费%' OR question ILIKE '%奖学金%' THEN '校园生活'
                ELSE '其他'
              END AS category,
              count
            FROM (
              SELECT
                left(regexp_replace(trim(content), '\s+', ' ', 'g'), 80) AS question,
                COUNT(*)::bigint AS count,
                max(created_at) AS latest_at
              FROM conversation_messages
              WHERE role = 'user'
                AND trim(content) <> ''
              GROUP BY left(regexp_replace(trim(content), '\s+', ' ', 'g'), 80)
            ) grouped
            ORDER BY count DESC, latest_at DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| {
                let count = row.get::<i64, _>("count");
                AdminTopQuestion {
                    question: row.get("question"),
                    category: row.get("category"),
                    count,
                    share: format_share(count, total_questions.max(1)),
                }
            })
            .collect())
    }

    async fn admin_major_attention(&self, limit: i64) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            r#"
            SELECT m.name, COUNT(DISTINCT cm.id)::bigint AS count
            FROM majors m
            JOIN conversation_messages cm
              ON cm.role = 'user' AND cm.content ILIKE '%' || m.name || '%'
            GROUP BY m.name
            ORDER BY count DESC, m.name
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| (row.get::<String, _>("name"), row.get::<i64, _>("count")))
            .collect())
    }

    async fn admin_user_question_count(&self, start_sql: &str, end_sql: &str) -> Result<i64> {
        let sql = format!(
            "SELECT COUNT(*)::bigint AS count FROM conversation_messages WHERE role = 'user' AND created_at >= {start_sql} AND created_at < {end_sql}"
        );
        Ok(sqlx::query(&sql).fetch_one(&self.pool).await?.get("count"))
    }

    async fn admin_distinct_conversation_count(
        &self,
        start_sql: &str,
        end_sql: &str,
    ) -> Result<i64> {
        let sql = format!(
            "SELECT COUNT(DISTINCT conversation_id)::bigint AS count FROM conversation_messages WHERE role = 'user' AND created_at >= {start_sql} AND created_at < {end_sql}"
        );
        Ok(sqlx::query(&sql).fetch_one(&self.pool).await?.get("count"))
    }

    async fn admin_recent_user_question_points(&self, days: i64) -> Result<Vec<i64>> {
        let days = days.clamp(1, 30);
        let rows = sqlx::query(
            r#"
            SELECT
              day,
              COALESCE(COUNT(m.id), 0)::bigint AS value
            FROM generate_series(current_date - (($1::int - 1) * interval '1 day'), current_date, interval '1 day') AS day
            LEFT JOIN conversation_messages m
              ON m.role = 'user'
             AND m.created_at >= day
             AND m.created_at < day + interval '1 day'
            GROUP BY day
            ORDER BY day
            "#,
        )
        .bind(days as i32)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|row| row.get("value")).collect())
    }

    pub async fn admin_list_conversations(
        &self,
        query: &str,
        page: i64,
        page_size: i64,
    ) -> Result<AdminConversationList> {
        let page = page.max(1);
        let page_size = page_size.clamp(1, 100);
        let offset = (page - 1) * page_size;
        let pattern = format!("%{}%", query.trim());
        let total = sqlx::query(
            r#"
            SELECT COUNT(DISTINCT c.id)::bigint AS total
            FROM conversations c
            WHERE $1 = '%%'
               OR c.id ILIKE $1
               OR c.session_key ILIKE $1
               OR EXISTS (
                 SELECT 1 FROM conversation_messages m
                 WHERE m.conversation_id = c.id AND m.content ILIKE $1
               )
            "#,
        )
        .bind(&pattern)
        .fetch_one(&self.pool)
        .await?
        .get::<i64, _>("total");

        let rows = sqlx::query(
            r#"
            SELECT
              c.id,
              COALESCE(p.name, NULLIF(c.province_code, ''), '未知') AS province,
              to_char(c.updated_at, 'YYYY-MM-DD HH24:MI') AS updated_at,
              COUNT(m.id)::bigint AS message_count,
              COALESCE(
                (
                  SELECT left(regexp_replace(trim(um.content), '\s+', ' ', 'g'), 120)
                  FROM conversation_messages um
                  WHERE um.conversation_id = c.id AND um.role = 'user'
                  ORDER BY um.created_at DESC
                  LIMIT 1
                ),
                ''
              ) AS last_message
            FROM conversations c
            LEFT JOIN provinces p ON p.code = c.province_code OR p.name = c.province_code
            LEFT JOIN conversation_messages m ON m.conversation_id = c.id
            WHERE $1 = '%%'
               OR c.id ILIKE $1
               OR c.session_key ILIKE $1
               OR EXISTS (
                 SELECT 1 FROM conversation_messages sm
                 WHERE sm.conversation_id = c.id AND sm.content ILIKE $1
               )
            GROUP BY c.id, p.name, c.province_code, c.updated_at
            ORDER BY c.updated_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(&pattern)
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(AdminConversationList {
            items: rows
                .into_iter()
                .map(|row| AdminConversationListItem {
                    id: row.get("id"),
                    province: row.get("province"),
                    updated_at: row.get("updated_at"),
                    message_count: row.get("message_count"),
                    status: "待审核".to_owned(),
                    manual_intervention: false,
                    last_message: row.get("last_message"),
                })
                .collect(),
            total,
            page,
            page_size,
        })
    }

    pub async fn admin_get_conversation_detail(
        &self,
        conversation_id: &str,
    ) -> Result<Option<AdminConversationDetail>> {
        let Some(history) = self.get_conversation_history(conversation_id).await? else {
            return Ok(None);
        };
        let row = sqlx::query(
            r#"
            SELECT COALESCE(p.name, NULLIF(c.province_code, ''), '未知') AS province
            FROM conversations c
            LEFT JOIN provinces p ON p.code = c.province_code OR p.name = c.province_code
            WHERE c.id = $1 OR c.session_key = $1
            LIMIT 1
            "#,
        )
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?;
        let province = row
            .map(|row| row.get::<String, _>("province"))
            .unwrap_or_else(|| "未知".to_owned());
        Ok(Some(AdminConversationDetail {
            id: history.id,
            province,
            status: "待审核".to_owned(),
            manual_intervention: false,
            message_count: history.messages.len(),
            messages: history.messages,
        }))
    }

    pub async fn admin_list_faqs(
        &self,
        query: &str,
        page: i64,
        page_size: i64,
    ) -> Result<AdminFaqList> {
        let page = page.max(1);
        let page_size = page_size.clamp(1, 100);
        let offset = (page - 1) * page_size;
        let pattern = format!("%{}%", query.trim());
        let total = sqlx::query(
            r#"
            SELECT COUNT(*)::bigint AS total
            FROM faq_knowledge
            WHERE $1 = '%%'
               OR question ILIKE $1
               OR answer ILIKE $1
               OR category ILIKE $1
            "#,
        )
        .bind(&pattern)
        .fetch_one(&self.pool)
        .await?
        .get::<i64, _>("total");
        let rows = sqlx::query(
            r#"
            SELECT
              fk.id,
              fk.question,
              fk.answer,
              fk.category,
              fk.source_label,
              CASE WHEN fk.status::text = 'PUBLISHED' THEN '启用' ELSE '禁用' END AS status,
              to_char(fk.updated_at, 'YYYY-MM-DD') AS updated_at,
              COALESCE(
                (
                  SELECT string_agg(value, '|')
                  FROM jsonb_array_elements_text(COALESCE(fk.tags, '[]'::jsonb)) AS value
                ),
                ''
              ) AS similar,
              COUNT(kc.id)::bigint AS hits
            FROM faq_knowledge fk
            LEFT JOIN knowledge_chunks kc ON kc.faq_knowledge_id = fk.id
            WHERE $1 = '%%'
               OR fk.question ILIKE $1
               OR fk.answer ILIKE $1
               OR fk.category ILIKE $1
            GROUP BY fk.id
            ORDER BY fk.updated_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(&pattern)
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(AdminFaqList {
            items: rows
                .into_iter()
                .map(|row| AdminFaqItem {
                    id: row.get("id"),
                    question: row.get("question"),
                    similar: row.get("similar"),
                    answer: row.get("answer"),
                    source: row.get::<String, _>("source_label"),
                    updated_at: row.get("updated_at"),
                    status: row.get("status"),
                    hits: row.get("hits"),
                })
                .collect(),
            total,
            page,
            page_size,
        })
    }

    pub async fn admin_create_faq(
        &self,
        question: &str,
        answer: &str,
        category: &str,
        tags: Vec<String>,
        status: &str,
        source_label: &str,
    ) -> Result<AdminFaqItem> {
        let id = format!("faq_admin_{}", Uuid::new_v4().simple());
        let status = normalize_faq_status(status);
        let tags = serde_json::Value::Array(
            tags.into_iter()
                .filter_map(|tag| {
                    let tag = tag.trim().to_owned();
                    (!tag.is_empty()).then_some(serde_json::Value::String(tag))
                })
                .collect(),
        );
        let row = sqlx::query(
            r#"
            INSERT INTO faq_knowledge
              (id, question, answer, category, tags, status, source_label, data_version, created_at, updated_at)
            VALUES
              ($1, $2, $3, $4, $5, $6::"FaqStatus", $7, 'admin-faq-v1', now(), now())
            RETURNING
              id,
              question,
              answer,
              category,
              source_label,
              CASE WHEN status::text = 'PUBLISHED' THEN '启用' ELSE '禁用' END AS status,
              to_char(updated_at, 'YYYY-MM-DD') AS updated_at,
              COALESCE(
                (
                  SELECT string_agg(value, '|')
                  FROM jsonb_array_elements_text(COALESCE(tags, '[]'::jsonb)) AS value
                ),
                ''
              ) AS similar,
              0::bigint AS hits
            "#,
        )
        .bind(&id)
        .bind(question.trim())
        .bind(answer.trim())
        .bind(category.trim())
        .bind(tags)
        .bind(status)
        .bind(source_label.trim())
        .fetch_one(&self.pool)
        .await?;

        self.admin_insert_audit_log(
            "create_faq",
            "faq_knowledge",
            Some(&id),
            "admin",
            serde_json::json!({ "status": status, "category": category }),
        )
        .await
        .ok();

        Ok(admin_faq_from_row(row))
    }

    pub async fn admin_update_faq(
        &self,
        id: &str,
        question: Option<&str>,
        answer: Option<&str>,
        category: Option<&str>,
        tags: Option<Vec<String>>,
        status: Option<&str>,
        source_label: Option<&str>,
    ) -> Result<Option<AdminFaqItem>> {
        let tags_value = tags.map(|tags| {
            serde_json::Value::Array(
                tags.into_iter()
                    .filter_map(|tag| {
                        let tag = tag.trim().to_owned();
                        (!tag.is_empty()).then_some(serde_json::Value::String(tag))
                    })
                    .collect(),
            )
        });
        let status = status.map(normalize_faq_status);
        let row = sqlx::query(
            r#"
            UPDATE faq_knowledge
            SET
              question = COALESCE(NULLIF($2, ''), question),
              answer = COALESCE(NULLIF($3, ''), answer),
              category = COALESCE(NULLIF($4, ''), category),
              tags = COALESCE($5, tags),
              status = COALESCE($6::"FaqStatus", status),
              source_label = COALESCE(NULLIF($7, ''), source_label),
              updated_at = now()
            WHERE id = $1
            RETURNING
              id,
              question,
              answer,
              category,
              source_label,
              CASE WHEN status::text = 'PUBLISHED' THEN '启用' ELSE '禁用' END AS status,
              to_char(updated_at, 'YYYY-MM-DD') AS updated_at,
              COALESCE(
                (
                  SELECT string_agg(value, '|')
                  FROM jsonb_array_elements_text(COALESCE(tags, '[]'::jsonb)) AS value
                ),
                ''
              ) AS similar,
              (
                SELECT COUNT(kc.id)::bigint
                FROM knowledge_chunks kc
                WHERE kc.faq_knowledge_id = faq_knowledge.id
              ) AS hits
            "#,
        )
        .bind(id)
        .bind(question.map(str::trim))
        .bind(answer.map(str::trim))
        .bind(category.map(str::trim))
        .bind(tags_value)
        .bind(status)
        .bind(source_label.map(str::trim))
        .fetch_optional(&self.pool)
        .await?;

        if row.is_some() {
            self.admin_insert_audit_log(
                "update_faq",
                "faq_knowledge",
                Some(id),
                "admin",
                serde_json::json!({ "status": status }),
            )
            .await
            .ok();
        }

        Ok(row.map(admin_faq_from_row))
    }

    pub async fn admin_list_knowledge_chunks(
        &self,
        query: &str,
        page: i64,
        page_size: i64,
    ) -> Result<AdminKnowledgeChunkList> {
        let page = page.max(1);
        let page_size = page_size.clamp(1, 100);
        let offset = (page - 1) * page_size;
        let pattern = format!("%{}%", query.trim());
        let total = sqlx::query(
            r#"
            SELECT COUNT(*)::bigint AS total
            FROM knowledge_chunks kc
            WHERE kc.data_version = 'official-pdf-knowledge-v2'
              AND (
                $1 = '%%'
                OR kc.title ILIKE $1
                OR kc.content ILIKE $1
                OR kc.metadata::text ILIKE $1
              )
            "#,
        )
        .bind(&pattern)
        .fetch_one(&self.pool)
        .await?
        .get::<i64, _>("total");
        let rows = sqlx::query(
            r#"
            SELECT
              kc.id,
              kc.title,
              left(regexp_replace(trim(kc.content), '\s+', ' ', 'g'), 180) AS excerpt,
              kc.source_type::text AS source_type,
              kc.metadata->>'documentKind' AS document_kind,
              kc.metadata->>'college' AS college,
              kc.metadata->>'majorName' AS major_name,
              to_char(kc.updated_at, 'YYYY-MM-DD') AS updated_at
            FROM knowledge_chunks kc
            WHERE kc.data_version = 'official-pdf-knowledge-v2'
              AND (
                $1 = '%%'
                OR kc.title ILIKE $1
                OR kc.content ILIKE $1
                OR kc.metadata::text ILIKE $1
              )
            ORDER BY kc.updated_at DESC, kc.chunk_index ASC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(&pattern)
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(AdminKnowledgeChunkList {
            items: rows
                .into_iter()
                .map(|row| AdminKnowledgeChunkItem {
                    id: row.get("id"),
                    title: row.try_get("title").ok(),
                    excerpt: row.get("excerpt"),
                    document_kind: row.try_get("document_kind").ok(),
                    college: row.try_get("college").ok(),
                    major_name: row.try_get("major_name").ok(),
                    source_type: row.get("source_type"),
                    updated_at: row.get("updated_at"),
                })
                .collect(),
            total,
            page,
            page_size,
        })
    }

    pub async fn admin_create_feedback(
        &self,
        conversation_id: Option<&str>,
        message_id: Option<&str>,
        feedback_type: &str,
        comment: Option<&str>,
        handled_by: Option<&str>,
        status: &str,
    ) -> Result<AdminFeedbackItem> {
        let feedback_type = normalize_feedback_type(feedback_type);
        let status = normalize_feedback_status(status);
        let id = format!("fb_{}", Uuid::new_v4().simple());
        let actor = handled_by
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("admin");
        let handled_by_db = match handled_by.map(str::trim).filter(|value| !value.is_empty()) {
            Some(candidate) => {
                let exists =
                    sqlx::query("SELECT EXISTS(SELECT 1 FROM admin_users WHERE id = $1) AS exists")
                        .bind(candidate)
                        .fetch_one(&self.pool)
                        .await?
                        .get::<bool, _>("exists");
                exists.then_some(candidate)
            }
            None => None,
        };
        let row = sqlx::query(
            r#"
            INSERT INTO feedback_records
              (id, conversation_id, message_id, feedback_type, comment, handled_by, status)
            VALUES
              ($1, $2, $3, $4::"FeedbackType", $5, $6, $7::"FeedbackStatus")
            RETURNING
              id,
              conversation_id,
              message_id,
              feedback_type::text AS feedback_type,
              comment,
              handled_by,
              status::text AS status,
              to_char(created_at, 'YYYY-MM-DD HH24:MI') AS created_at
            "#,
        )
        .bind(&id)
        .bind(conversation_id)
        .bind(message_id)
        .bind(feedback_type)
        .bind(comment)
        .bind(handled_by_db)
        .bind(status)
        .fetch_one(&self.pool)
        .await?;

        self.admin_insert_audit_log(
            "create_feedback",
            "feedback_records",
            Some(&id),
            actor,
            serde_json::json!({
                "conversationId": conversation_id,
                "feedbackType": feedback_type,
                "status": status
            }),
        )
        .await
        .ok();

        Ok(AdminFeedbackItem {
            id: row.get("id"),
            conversation_id: row.try_get("conversation_id").ok(),
            message_id: row.try_get("message_id").ok(),
            feedback_type: feedback_type_to_frontend(row.get::<String, _>("feedback_type")),
            comment: row.try_get("comment").ok(),
            handled_by: row.try_get("handled_by").ok(),
            status: feedback_status_to_frontend(row.get::<String, _>("status")),
            created_at: row.get("created_at"),
        })
    }

    pub async fn admin_list_tickets(
        &self,
        query: &str,
        status: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> Result<AdminTicketList> {
        self.ensure_admin_ops_schema().await?;
        let page = page.max(1);
        let page_size = page_size.clamp(1, 100);
        let offset = (page - 1) * page_size;
        let pattern = format!("%{}%", query.trim());
        let status = status
            .map(normalize_ticket_status)
            .filter(|value| value != "all");

        let total = sqlx::query(
            r#"
            SELECT COUNT(*)::bigint AS total
            FROM admin_tickets
            WHERE ($1 = '%%' OR id ILIKE $1 OR name ILIKE $1 OR phone ILIKE $1 OR email ILIKE $1 OR province ILIKE $1 OR content ILIKE $1)
              AND ($2::text IS NULL OR status = $2)
            "#,
        )
        .bind(&pattern)
        .bind(status.as_deref())
        .fetch_one(&self.pool)
        .await?
        .get("total");

        let rows = sqlx::query(
            r#"
            SELECT
              id,
              name,
              phone,
              email,
              province,
              content,
              status,
              priority,
              to_char(created_at, 'YYYY-MM-DD HH24:MI') AS created_at,
              to_char(updated_at, 'YYYY-MM-DD HH24:MI') AS updated_at,
              handled_by,
              resolution
            FROM admin_tickets
            WHERE ($1 = '%%' OR id ILIKE $1 OR name ILIKE $1 OR phone ILIKE $1 OR email ILIKE $1 OR province ILIKE $1 OR content ILIKE $1)
              AND ($2::text IS NULL OR status = $2)
            ORDER BY
              CASE priority WHEN '高' THEN 0 WHEN '中' THEN 1 ELSE 2 END,
              CASE status WHEN '待处理' THEN 0 WHEN '处理中' THEN 1 WHEN '已办结' THEN 2 ELSE 3 END,
              created_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(&pattern)
        .bind(status.as_deref())
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(AdminTicketList {
            items: rows.into_iter().map(admin_ticket_from_row).collect(),
            total,
            page,
            page_size,
        })
    }

    pub async fn create_public_ticket(
        &self,
        name: Option<&str>,
        phone: &str,
        email: Option<&str>,
        province: &str,
        content: &str,
    ) -> Result<AdminTicketItem> {
        self.ensure_admin_ops_schema().await?;
        let id = format!("T-{}", Uuid::new_v4().simple());
        let name = name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("匿名用户");
        let email = email.map(str::trim).filter(|value| !value.is_empty());

        let row = sqlx::query(
            r#"
            INSERT INTO admin_tickets (
              id,
              name,
              phone,
              email,
              province,
              content,
              status,
              priority
            )
            VALUES ($1, $2, $3, $4, $5, $6, '待处理', '中')
            RETURNING
              id,
              name,
              phone,
              email,
              province,
              content,
              status,
              priority,
              to_char(created_at, 'YYYY-MM-DD HH24:MI') AS created_at,
              to_char(updated_at, 'YYYY-MM-DD HH24:MI') AS updated_at,
              handled_by,
              resolution
            "#,
        )
        .bind(id)
        .bind(name)
        .bind(phone.trim())
        .bind(email)
        .bind(province.trim())
        .bind(content.trim())
        .fetch_one(&self.pool)
        .await?;

        Ok(admin_ticket_from_row(row))
    }

    pub async fn admin_update_ticket(
        &self,
        ticket_id: &str,
        status: Option<&str>,
        resolution: Option<&str>,
        handled_by: Option<&str>,
    ) -> Result<Option<AdminTicketItem>> {
        self.ensure_admin_ops_schema().await?;
        let status = status.map(normalize_ticket_status);
        let row = sqlx::query(
            r#"
            UPDATE admin_tickets
            SET
              status = COALESCE($2, status),
              resolution = COALESCE($3, resolution),
              handled_by = COALESCE($4, handled_by),
              updated_at = now()
            WHERE id = $1
            RETURNING
              id,
              name,
              phone,
              email,
              province,
              content,
              status,
              priority,
              to_char(created_at, 'YYYY-MM-DD HH24:MI') AS created_at,
              to_char(updated_at, 'YYYY-MM-DD HH24:MI') AS updated_at,
              handled_by,
              resolution
            "#,
        )
        .bind(ticket_id)
        .bind(status.as_deref())
        .bind(resolution)
        .bind(handled_by)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(status) = status.as_deref() {
            self.admin_insert_audit_log(
                "update_ticket",
                "admin_tickets",
                Some(ticket_id),
                handled_by.unwrap_or("admin"),
                serde_json::json!({ "status": status }),
            )
            .await
            .ok();
        }

        Ok(row.map(admin_ticket_from_row))
    }

    pub async fn admin_get_settings(&self) -> Result<AdminSettings> {
        self.ensure_admin_ops_schema().await?;
        let rows = sqlx::query(
            r#"
            SELECT key, value, to_char(updated_at, 'YYYY-MM-DD HH24:MI') AS updated_at
            FROM admin_settings
            WHERE key IN ('welcome_message', 'fallback_message')
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut settings = default_admin_settings();
        for row in rows {
            let key = row.get::<String, _>("key");
            let value = row.get::<String, _>("value");
            let updated_at = row.try_get::<String, _>("updated_at").ok();
            match key.as_str() {
                "welcome_message" => settings.welcome_message = value,
                "fallback_message" => settings.fallback_message = value,
                _ => {}
            }
            settings.updated_at = updated_at.or(settings.updated_at);
        }
        Ok(settings)
    }

    pub async fn admin_update_settings(
        &self,
        welcome_message: &str,
        fallback_message: &str,
        updated_by: Option<&str>,
    ) -> Result<AdminSettings> {
        self.ensure_admin_ops_schema().await?;
        let actor = updated_by.unwrap_or("admin");
        for (key, value) in [
            ("welcome_message", welcome_message.trim()),
            ("fallback_message", fallback_message.trim()),
        ] {
            sqlx::query(
                r#"
                INSERT INTO admin_settings (key, value, updated_by, updated_at)
                VALUES ($1, $2, $3, now())
                ON CONFLICT (key)
                DO UPDATE SET value = EXCLUDED.value, updated_by = EXCLUDED.updated_by, updated_at = now()
                "#,
            )
            .bind(key)
            .bind(value)
            .bind(actor)
            .execute(&self.pool)
            .await?;
        }

        self.admin_insert_audit_log(
            "update_settings",
            "admin_settings",
            None,
            actor,
            serde_json::json!({ "keys": ["welcome_message", "fallback_message"] }),
        )
        .await
        .ok();

        self.admin_get_settings().await
    }

    pub async fn admin_list_audit_logs(
        &self,
        page: i64,
        page_size: i64,
    ) -> Result<AdminAuditLogList> {
        self.ensure_admin_ops_schema().await?;
        let page = page.max(1);
        let page_size = page_size.clamp(1, 100);
        let offset = (page - 1) * page_size;
        let total = sqlx::query("SELECT COUNT(*)::bigint AS total FROM admin_audit_logs")
            .fetch_one(&self.pool)
            .await?
            .get("total");
        let rows = sqlx::query(
            r#"
            SELECT
              id,
              action,
              target_type,
              target_id,
              actor,
              detail,
              to_char(created_at, 'YYYY-MM-DD HH24:MI') AS created_at
            FROM admin_audit_logs
            ORDER BY created_at DESC
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(AdminAuditLogList {
            items: rows
                .into_iter()
                .map(|row| AdminAuditLogItem {
                    id: row.get("id"),
                    action: row.get("action"),
                    target_type: row.get("target_type"),
                    target_id: row.try_get("target_id").ok(),
                    actor: row.get("actor"),
                    detail: row.get("detail"),
                    created_at: row.get("created_at"),
                })
                .collect(),
            total,
            page,
            page_size,
        })
    }

    async fn ensure_admin_ops_schema(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS admin_tickets (
              id text PRIMARY KEY,
              name text NOT NULL DEFAULT '匿名用户',
              phone text,
              email text,
              province text NOT NULL DEFAULT '未知',
              content text NOT NULL,
              status text NOT NULL DEFAULT '待处理',
              priority text NOT NULL DEFAULT '中',
              handled_by text,
              resolution text,
              created_at timestamp without time zone NOT NULL DEFAULT now(),
              updated_at timestamp without time zone NOT NULL DEFAULT now()
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("ALTER TABLE admin_tickets ADD COLUMN IF NOT EXISTS email text")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS admin_settings (
              key text PRIMARY KEY,
              value text NOT NULL,
              updated_by text,
              updated_at timestamp without time zone NOT NULL DEFAULT now()
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS admin_audit_logs (
              id text PRIMARY KEY,
              action text NOT NULL,
              target_type text NOT NULL,
              target_id text,
              actor text NOT NULL DEFAULT 'admin',
              detail jsonb NOT NULL DEFAULT '{}'::jsonb,
              created_at timestamp without time zone NOT NULL DEFAULT now()
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn admin_insert_audit_log(
        &self,
        action: &str,
        target_type: &str,
        target_id: Option<&str>,
        actor: &str,
        detail: Value,
    ) -> Result<()> {
        self.ensure_admin_ops_schema().await?;
        sqlx::query(
            r#"
            INSERT INTO admin_audit_logs (id, action, target_type, target_id, actor, detail)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(format!("audit_{}", Uuid::new_v4().simple()))
        .bind(action)
        .bind(target_type)
        .bind(target_id)
        .bind(actor)
        .bind(detail)
        .execute(&self.pool)
        .await?;
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

    pub async fn admin_evaluation_summary_snapshot(&self) -> Result<AdminEvaluationSummarySnapshot> {
        let (
            (total_count, province_count, avg_prob),
            provinces,
            daily_trend,
            subjects,
            scores,
            top_majors,
        ) = tokio::try_join!(
            self.fetch_evaluation_stats(),
            self.fetch_evaluation_provinces(),
            self.fetch_evaluation_daily_trend(),
            self.fetch_evaluation_subjects(),
            self.fetch_evaluation_scores(),
            self.fetch_evaluation_top_majors(),
        )?;

        let updated_at = sqlx::query("SELECT to_char(now(), 'YYYY-MM-DD HH24:MI') AS updated_at")
            .fetch_one(&self.pool)
            .await?
            .get::<String, _>("updated_at");

        let stats = vec![
            AdminStat {
                label: "总测评量".to_owned(),
                value: format_number(total_count),
                delta: None,
                tone: Some("blue".to_owned()),
            },
            AdminStat {
                label: "覆盖省份".to_owned(),
                value: format_number(province_count),
                delta: None,
                tone: Some("green".to_owned()),
            },
            AdminStat {
                label: "平均录取概率".to_owned(),
                value: format!("{:.1}%", avg_prob),
                delta: None,
                tone: Some("amber".to_owned()),
            },
            AdminStat {
                label: "最热门专业".to_owned(),
                value: top_majors.first().map(|(m, _)| m.clone()).unwrap_or_else(|| "暂无".to_owned()),
                delta: None,
                tone: Some("purple".to_owned()),
            },
        ];

        Ok(AdminEvaluationSummarySnapshot {
            updated_at,
            stats,
            province_bars: provinces,
            daily_trend,
            subject_distribution: subjects,
            score_distribution: scores,
            top_majors,
        })
    }

    pub async fn admin_evaluation_list(
        &self,
        query: &str,
        page: i64,
        page_size: i64,
    ) -> Result<AdminEvaluationList> {
        let page = page.max(1);
        let page_size = page_size.clamp(1, 100);
        let offset = (page - 1) * page_size;
        let pattern = format!("%{}%", query.trim());

        let total = sqlx::query(
            r#"
            SELECT COUNT(*)::bigint AS total
            FROM conversation_messages
            WHERE structured_payload->>'type' = 'probability_assessment'
              AND (
                $1 = '%%'
                OR conversation_id ILIKE $1
                OR COALESCE(structured_payload->'assessment'->'province'->>'name', structured_payload->'assessment'->>'province', '') ILIKE $1
                OR COALESCE(structured_payload->'assessment'->'major'->>'name', structured_payload->'assessment'->>'major', '') ILIKE $1
                OR COALESCE(structured_payload->'assessment'->>'level', '') ILIKE $1
                OR COALESCE(structured_payload->'assessment'->>'summary', '') ILIKE $1
              )
            "#,
        )
        .bind(&pattern)
        .fetch_one(&self.pool)
        .await?
        .get::<i64, _>("total");

        let rows = sqlx::query(
            r#"
            SELECT
              id,
              conversation_id,
              COALESCE(structured_payload->'assessment'->'province'->>'name', structured_payload->'assessment'->>'province', '未知') AS province,
              COALESCE(structured_payload->'assessment'->>'subjectType', '未知') AS subject_type,
              COALESCE((structured_payload->'assessment'->>'score')::bigint, 0) AS score,
              COALESCE((structured_payload->'assessment'->>'rank')::bigint, 0) AS rank,
              COALESCE(structured_payload->'assessment'->'major'->>'name', structured_payload->'assessment'->>'major', '未知') AS major_name,
              COALESCE((structured_payload->'assessment'->>'probability')::bigint, 0) AS probability,
              COALESCE(structured_payload->'assessment'->>'level', '未知') AS level,
              COALESCE(structured_payload->'assessment'->>'summary', '') AS summary,
              to_char(created_at, 'YYYY-MM-DD HH24:MI:SS') AS created_at
            FROM conversation_messages
            WHERE structured_payload->>'type' = 'probability_assessment'
              AND (
                $1 = '%%'
                OR conversation_id ILIKE $1
                OR COALESCE(structured_payload->'assessment'->'province'->>'name', structured_payload->'assessment'->>'province', '') ILIKE $1
                OR COALESCE(structured_payload->'assessment'->'major'->>'name', structured_payload->'assessment'->>'major', '') ILIKE $1
                OR COALESCE(structured_payload->'assessment'->>'level', '') ILIKE $1
                OR COALESCE(structured_payload->'assessment'->>'summary', '') ILIKE $1
              )
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(&pattern)
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let items = rows
            .into_iter()
            .map(|row| AdminEvaluationListItem {
                id: row.get("id"),
                conversation_id: row.get("conversation_id"),
                province: row.get("province"),
                subject_type: row.get("subject_type"),
                score: row.get("score"),
                rank: row.get("rank"),
                major_name: row.get("major_name"),
                probability: row.get("probability"),
                level: row.get("level"),
                summary: row.get("summary"),
                created_at: row.get("created_at"),
            })
            .collect();

        Ok(AdminEvaluationList { items, total })
    }

    // ---- Helper methods for evaluation statistics and distributions ----

    async fn fetch_evaluation_stats(&self) -> Result<(i64, i64, f64)> {
        let row = sqlx::query(
            r#"
            SELECT 
              COUNT(*)::bigint AS total_count,
              COUNT(DISTINCT COALESCE(structured_payload->'assessment'->'province'->>'name', structured_payload->'assessment'->>'province', '未知'))::bigint AS province_count,
              COALESCE(AVG((structured_payload->'assessment'->>'probability')::double precision), 0.0)::double precision AS avg_prob
            FROM conversation_messages 
            WHERE structured_payload->>'type' = 'probability_assessment'
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok((
            row.get::<i64, _>("total_count"),
            row.get::<i64, _>("province_count"),
            row.get::<f64, _>("avg_prob"),
        ))
    }

    async fn fetch_evaluation_provinces(&self) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            r#"
            SELECT 
              REPLACE(COALESCE(structured_payload->'assessment'->'province'->>'name', structured_payload->'assessment'->>'province', '未知'), '省', '') AS province,
              COUNT(*)::bigint AS count
            FROM conversation_messages
            WHERE structured_payload->>'type' = 'probability_assessment'
            GROUP BY province
            ORDER BY count DESC, province
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.get::<String, _>("province"), row.get::<i64, _>("count")))
            .collect())
    }

    async fn fetch_evaluation_daily_trend(&self) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            r#"
            SELECT
              to_char(d.day, 'YYYY-MM-DD') AS label,
              COALESCE(COUNT(m.id), 0)::bigint AS value
            FROM (
              SELECT generate_series(
                COALESCE(MAX(created_at)::date - interval '29 days', current_date - interval '29 days'),
                COALESCE(MAX(created_at)::date, current_date),
                interval '1 day'
              )::date AS day
              FROM conversation_messages
              WHERE structured_payload->>'type' = 'probability_assessment'
            ) d
            LEFT JOIN conversation_messages m
              ON m.structured_payload->>'type' = 'probability_assessment'
             AND m.created_at::date = d.day
            GROUP BY d.day
            ORDER BY d.day ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.get::<String, _>("label"), row.get::<i64, _>("value")))
            .collect())
    }

    async fn fetch_evaluation_subjects(&self) -> Result<Vec<AdminChartDatum>> {
        let rows = sqlx::query(
            r#"
            SELECT 
              COALESCE(structured_payload->'assessment'->>'subjectType', '未知') AS name,
              COUNT(*)::bigint AS value
            FROM conversation_messages
            WHERE structured_payload->>'type' = 'probability_assessment'
            GROUP BY name
            ORDER BY value DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AdminChartDatum {
                name: row.get("name"),
                value: row.get("value"),
            })
            .collect())
    }

    async fn fetch_evaluation_scores(&self) -> Result<Vec<AdminChartDatum>> {
        let rows = sqlx::query(
            r#"
            SELECT 
              CASE 
                WHEN (structured_payload->'assessment'->>'score') IS NULL THEN '未填写'
                WHEN (structured_payload->'assessment'->>'score')::integer < 400 THEN '400分以下'
                WHEN (structured_payload->'assessment'->>'score')::integer >= 400 AND (structured_payload->'assessment'->>'score')::integer < 500 THEN '400-500分'
                WHEN (structured_payload->'assessment'->>'score')::integer >= 500 AND (structured_payload->'assessment'->>'score')::integer < 600 THEN '500-600分'
                ELSE '600分以上'
              END AS name,
              COUNT(*)::bigint AS value
            FROM conversation_messages
            WHERE structured_payload->>'type' = 'probability_assessment'
            GROUP BY name
            ORDER BY value DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AdminChartDatum {
                name: row.get("name"),
                value: row.get("value"),
            })
            .collect())
    }

    async fn fetch_evaluation_top_majors(&self) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            r#"
            SELECT 
              COALESCE(structured_payload->'assessment'->'major'->>'name', structured_payload->'assessment'->>'major', '未知') AS name,
              COUNT(*)::bigint AS count
            FROM conversation_messages
            WHERE structured_payload->>'type' = 'probability_assessment'
            GROUP BY name
            ORDER BY count DESC, name
            LIMIT 10
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.get::<String, _>("name"), row.get::<i64, _>("count")))
            .collect())
    }
}

fn read_env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn format_number(value: i64) -> String {
    let raw = value.abs().to_string();
    let mut formatted = String::new();
    for (index, ch) in raw.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(ch);
    }
    let mut result = formatted.chars().rev().collect::<String>();
    if value < 0 {
        result.insert(0, '-');
    }
    result
}

fn format_share(count: i64, total: i64) -> String {
    if total <= 0 {
        return "0.0%".to_owned();
    }
    format!("{:.1}%", count as f64 * 100.0 / total as f64)
}

fn normalize_feedback_type(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "helpful" | "有帮助" => "HELPFUL",
        "manual-fix" | "manual_fix" | "manualfix" | "人工纠错" => "MANUAL_FIX",
        _ => "INCORRECT",
    }
}

fn normalize_faq_status(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "published" | "publish" | "启用" | "已发布" => "PUBLISHED",
        _ => "DRAFT",
    }
}

fn admin_faq_from_row(row: sqlx::postgres::PgRow) -> AdminFaqItem {
    AdminFaqItem {
        id: row.get("id"),
        question: row.get("question"),
        similar: row.get("similar"),
        answer: row.get("answer"),
        source: row.get::<String, _>("source_label"),
        updated_at: row.get("updated_at"),
        status: row.get("status"),
        hits: row.get("hits"),
    }
}

fn normalize_feedback_status(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "resolved" | "已解决" | "已办结" => "RESOLVED",
        _ => "OPEN",
    }
}

fn feedback_type_to_frontend(value: String) -> String {
    match value.as_str() {
        "HELPFUL" => "helpful",
        "MANUAL_FIX" => "manual-fix",
        _ => "incorrect",
    }
    .to_owned()
}

fn feedback_status_to_frontend(value: String) -> String {
    match value.as_str() {
        "RESOLVED" => "resolved",
        _ => "open",
    }
    .to_owned()
}

fn normalize_ticket_status(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "working" | "处理中" => "处理中",
        "resolved" | "done" | "已办结" => "已办结",
        "closed" | "已关闭" => "已关闭",
        "all" | "全部" => "all",
        _ => "待处理",
    }
    .to_owned()
}

fn admin_ticket_from_row(row: sqlx::postgres::PgRow) -> AdminTicketItem {
    AdminTicketItem {
        id: row.get("id"),
        name: row.get("name"),
        phone: row.try_get("phone").ok(),
        email: row.try_get("email").ok(),
        province: row.get("province"),
        content: row.get("content"),
        status: row.get("status"),
        priority: row.get("priority"),
        created_at: row.get("created_at"),
        updated_at: row.try_get("updated_at").ok(),
        handled_by: row.try_get("handled_by").ok(),
        resolution: row.try_get("resolution").ok(),
    }
}

fn default_admin_settings() -> AdminSettings {
    AdminSettings {
        welcome_message: "您好，欢迎来到哈尔滨师范大学！我是您的招生咨询助手「沐阳」，很高兴为您服务。请问有什么可以帮助您的吗？".to_owned(),
        fallback_message: "抱歉，我暂时无法回答这个问题。建议您拨打招生咨询电话：0451-88060678，或者提交人工留言，我们会尽快为您解答。".to_owned(),
        updated_at: None,
    }
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
