use agent_runtime::{
    CompressionConfig, RequiredToolGuard, RuntimeContext, RuntimeOutput, compress_context,
};
use anyhow::{Context, Result};
use db::{Database, memory_from_profile};
use domain::{
    ChatCitation, ChatIntent, ChatReply, ChatRequest, ChatStructuredResult, ConversationMessage,
    ResolvedMemory,
};
use futures::StreamExt;
use llm::{LlmMessage, LlmProvider, MessageRole, OpenAiCompatibleClient};
use probability::{
    ProbabilityEngineInput, ProbabilityPlanHistoryItem, ProbabilityScoreHistoryItem,
    ProbabilitySourceMode, calculate_admission_probability,
};
use retrieval::{
    RetrievalIntent, RetrievalService, extract_college_major_catalog_query,
    render_college_major_answer, render_knowledge_answer, render_major_disambiguation,
    route_message,
};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AdmissionsAgent {
    db: Database,
    retrieval: RetrievalService,
    compression_config: CompressionConfig,
    llm: Option<OpenAiCompatibleClient>,
    turn_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

struct PreparedChatTurn {
    started_at: Instant,
    conversation_id: String,
    user_message: String,
    memory: ResolvedMemory,
    draft_reply: String,
    structured_result: ChatStructuredResult,
    citations: Vec<ChatCitation>,
    route_intent: ChatIntent,
    tool_call_count: usize,
    trace: Vec<domain::AgentTraceStep>,
    compressed: agent_runtime::CompressedContext,
}

impl AdmissionsAgent {
    pub fn new(db: Database) -> Self {
        let retrieval = RetrievalService::new(db.clone());
        Self {
            db,
            retrieval,
            compression_config: CompressionConfig::default(),
            llm: OpenAiCompatibleClient::from_env_for_synthesis(),
            turn_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn chat(&self, input: ChatRequest) -> Result<ChatReply> {
        let conversation_id = self
            .db
            .get_or_create_conversation(input.conversation_id.as_deref())
            .await
            .context("failed to get or create conversation")?;
        let turn_lock = self.conversation_turn_lock(&conversation_id).await;
        let _turn_guard = turn_lock.lock_owned().await;
        let mut prepared = self.prepare_turn(conversation_id, input).await?;

        let mut final_reply = prepared.draft_reply.clone();
        let mut model_call_count = 0usize;
        let mut synthesis_used = false;
        let mut llm_model = self.llm.as_ref().map(|client| client.model().to_owned());
        if should_synthesize(&prepared.structured_result) {
            match self
                .synthesize_reply(
                    &prepared.user_message,
                    &final_reply,
                    &prepared.structured_result,
                    &prepared.citations,
                    &prepared.memory,
                    &prepared.compressed,
                )
                .await
            {
                Ok(Some(synthesized)) => {
                    model_call_count = 1;
                    synthesis_used = true;
                    final_reply = synthesized;
                    prepared.trace.push(domain::AgentTraceStep {
                        step: 2,
                        node: "llm_synthesis".to_owned(),
                        tool_name: None,
                        duration_ms: None,
                        error: None,
                    });
                }
                Ok(None) => {
                    llm_model = None;
                }
                Err(error) => {
                    tracing::warn!(error = %error, "llm synthesis failed; falling back to draft reply");
                    prepared.trace.push(domain::AgentTraceStep {
                        step: 2,
                        node: "llm_synthesis".to_owned(),
                        tool_name: None,
                        duration_ms: None,
                        error: Some(error.to_string()),
                    });
                }
            }
        }

        self.finish_turn(
            prepared,
            final_reply,
            model_call_count,
            llm_model,
            synthesis_used,
        )
        .await
    }

    pub async fn chat_stream_with_deltas<F, Fut>(
        &self,
        input: ChatRequest,
        mut on_delta: F,
    ) -> Result<ChatReply>
    where
        F: FnMut(String, String) -> Fut + Send,
        Fut: Future<Output = bool> + Send,
    {
        let conversation_id = self
            .db
            .get_or_create_conversation(input.conversation_id.as_deref())
            .await
            .context("failed to get or create conversation")?;
        let turn_lock = self.conversation_turn_lock(&conversation_id).await;
        let _turn_guard = turn_lock.lock_owned().await;
        let mut prepared = self.prepare_turn(conversation_id, input).await?;

        let mut final_reply = prepared.draft_reply.clone();
        let mut model_call_count = 0usize;
        let mut synthesis_used = false;
        let mut llm_model = self.llm.as_ref().map(|client| client.model().to_owned());
        let mut emitted_any = false;

        if should_synthesize(&prepared.structured_result) {
            if let Some(llm) = &self.llm {
                match self.synthesis_messages(
                    &prepared.user_message,
                    &final_reply,
                    &prepared.structured_result,
                    &prepared.citations,
                    &prepared.memory,
                    &prepared.compressed,
                ) {
                    Ok(messages) => match llm.stream_complete(&messages).await {
                        Ok(mut stream) => {
                            let mut streamed = String::new();
                            let mut stream_error = None;
                            while let Some(delta_result) = stream.next().await {
                                match delta_result {
                                    Ok(delta) => {
                                        if delta.is_empty() {
                                            continue;
                                        }
                                        streamed.push_str(&delta);
                                        emitted_any = true;
                                        if !on_delta(prepared.conversation_id.clone(), delta).await
                                        {
                                            break;
                                        }
                                    }
                                    Err(error) => {
                                        stream_error = Some(error);
                                        break;
                                    }
                                }
                            }

                            if let Some(error) = stream_error {
                                tracing::warn!(error = %error, "llm streaming synthesis interrupted");
                                prepared.trace.push(domain::AgentTraceStep {
                                    step: 2,
                                    node: "llm_stream_synthesis".to_owned(),
                                    tool_name: None,
                                    duration_ms: None,
                                    error: Some(error.to_string()),
                                });
                                if streamed.is_empty() {
                                    emitted_any = false;
                                } else {
                                    final_reply = streamed;
                                    model_call_count = 1;
                                    synthesis_used = true;
                                }
                            } else if !streamed.trim().is_empty() {
                                final_reply = streamed;
                                model_call_count = 1;
                                synthesis_used = true;
                                prepared.trace.push(domain::AgentTraceStep {
                                    step: 2,
                                    node: "llm_stream_synthesis".to_owned(),
                                    tool_name: None,
                                    duration_ms: None,
                                    error: None,
                                });
                            } else {
                                emitted_any = false;
                            }
                        }
                        Err(error) => {
                            tracing::warn!(error = %error, "llm streaming synthesis failed; falling back to draft reply");
                            prepared.trace.push(domain::AgentTraceStep {
                                step: 2,
                                node: "llm_stream_synthesis".to_owned(),
                                tool_name: None,
                                duration_ms: None,
                                error: Some(error.to_string()),
                            });
                        }
                    },
                    Err(error) => {
                        tracing::warn!(error = %error, "failed to build synthesis messages");
                        prepared.trace.push(domain::AgentTraceStep {
                            step: 2,
                            node: "llm_stream_synthesis".to_owned(),
                            tool_name: None,
                            duration_ms: None,
                            error: Some(error.to_string()),
                        });
                    }
                }
            } else {
                llm_model = None;
            }
        }

        let emitted_reply = final_reply.clone();
        final_reply = finalize_reply(final_reply, &prepared.structured_result, &prepared.memory);
        if !emitted_any {
            for chunk in chunk_reply_text(&final_reply) {
                if !on_delta(prepared.conversation_id.clone(), chunk).await {
                    break;
                }
            }
        } else if final_reply != emitted_reply {
            let extra = final_reply
                .strip_prefix(&emitted_reply)
                .unwrap_or(&final_reply)
                .to_owned();
            if !extra.is_empty() {
                let _ = on_delta(prepared.conversation_id.clone(), extra).await;
            }
        }

        self.finish_turn(
            prepared,
            final_reply,
            model_call_count,
            llm_model,
            synthesis_used,
        )
        .await
    }

    async fn prepare_turn(
        &self,
        conversation_id: String,
        input: ChatRequest,
    ) -> Result<PreparedChatTurn> {
        let started_at = Instant::now();
        let history = self
            .db
            .get_conversation_recent_history(&conversation_id, conversation_history_window())
            .await?
            .map(|history| history.messages)
            .unwrap_or_default();
        let mut memory = memory_from_profile(input.profile.as_ref());
        enrich_memory_from_history(&mut memory, &history);
        enrich_memory_from_message(&mut memory, &input.message);
        self.resolve_major_from_message(&mut memory, &input.message)
            .await?;
        let last_assistant_structured = latest_assistant_structured(&history);

        self.db
            .append_message(&conversation_id, "user", &input.message, None, &[])
            .await?;

        let route = apply_contextual_route(route_message(&input.message), &input.message, &memory);
        let route_intent = to_chat_intent(&route.intent);
        let score_query_year =
            resolve_admission_score_year_for_turn(&input.message, &history, &route.intent);
        let compressed =
            compress_context(&history, &input.message, &memory, &self.compression_config);
        let mut trace = vec![domain::AgentTraceStep {
            step: 0,
            node: "router".to_owned(),
            tool_name: None,
            duration_ms: None,
            error: None,
        }];

        let combined_plan = combined_request_plan(
            &input.message,
            &memory,
            &route.intent,
            last_assistant_structured,
        );
        let (reply, structured_result, citations, tool_call_count) = if let Some(plan) =
            combined_plan
        {
            self.handle_combined_request(&input.message, &memory, plan, score_query_year)
                .await?
        } else {
            match route.intent {
                RetrievalIntent::Greeting => {
                    let structured = ChatStructuredResult::Greeting {
                        message: render_greeting_answer(&input.message),
                    };
                    let reply = render_greeting_answer(&input.message);
                    (reply, structured, Vec::new(), 0)
                }
                RetrievalIntent::KnowledgeAnswer => {
                    if asks_province_admission_major_list(&input.message) {
                        self.handle_province_major_list_request(&input.message, &memory)
                            .await?
                    } else if asks_major_admission_province_list(&input.message) {
                        self.handle_major_province_list_request(&input.message)
                            .await?
                    } else if let Some(college_name) =
                        extract_college_major_catalog_query(&input.message)
                    {
                        let majors = self.retrieval.list_college_majors(&college_name).await?;
                        let (reply, structured, citations) =
                            render_college_major_answer(&college_name, &majors);
                        (reply, structured, citations, 1)
                    } else if asks_major_group_without_college(&input.message) {
                        let candidates = self
                            .retrieval
                            .search_major_candidates(&input.message, 8)
                            .await?;
                        let structured = render_major_disambiguation(
                            &input.message,
                            candidates,
                            ChatIntent::ScoreQuery,
                        );
                        let reply = render_major_disambiguation_reply(&structured);
                        (reply, structured, Vec::new(), 1)
                    } else {
                        let knowledge_query = contextual_knowledge_query_with_history(
                            &input.message,
                            &memory,
                            &history,
                        );
                        let mut result = self
                            .retrieve_knowledge_with_memory_focus(&knowledge_query, &memory)
                            .await?;
                        backfill_contextual_training_plan_chunks(
                            &mut result.structured_result,
                            &history,
                        );
                        RequiredToolGuard {
                            intent: ChatIntent::KnowledgeAnswer,
                            has_evidence: has_knowledge_evidence(&result.structured_result),
                        }
                        .validate()
                        .or_else(|_| {
                            // The guard blocks direct hallucination; synthesis still returns a boundary-aware
                            // answer when tools ran but no reliable evidence was found.
                            Ok::<(), anyhow::Error>(())
                        })?;
                        let reply = render_knowledge_answer(&result.structured_result);
                        (reply, result.structured_result, result.citations, 1)
                    }
                }
                RetrievalIntent::ScoreQuery => {
                    let missing = missing_score_fields(&memory);
                    if !missing.is_empty() {
                        let structured = ChatStructuredResult::FollowUp {
                            pending_intent: ChatIntent::ScoreQuery,
                            missing_fields: missing.clone(),
                            collected_profile: memory.clone(),
                        };
                        (
                            render_follow_up(&missing, &memory),
                            structured,
                            Vec::new(),
                            0,
                        )
                    } else {
                        let structured = self
                            .query_scores_from_memory(&memory, score_query_year)
                            .await?;
                        let citations = citations_from_structured_result(&structured);
                        let reply = render_score_answer(&structured);
                        (reply, structured, citations, 1)
                    }
                }
                RetrievalIntent::ProbabilityAssessment => {
                    let missing = effective_probability_missing_fields(&input.message, &memory);
                    let mut preloaded_score_records = None;
                    let missing = if missing.len() == 1
                        && missing.first().is_some_and(|field| field == "subjectType")
                    {
                        let score_records = self
                            .query_scores_from_memory(&memory, score_query_year)
                            .await?;
                        if score_query_uses_only_unspecified_subject(&score_records) {
                            preloaded_score_records = Some(score_records);
                            Vec::new()
                        } else {
                            missing
                        }
                    } else {
                        missing
                    };
                    if !missing.is_empty() {
                        let structured = ChatStructuredResult::FollowUp {
                            pending_intent: ChatIntent::ProbabilityAssessment,
                            missing_fields: missing.clone(),
                            collected_profile: memory.clone(),
                        };
                        (
                            render_follow_up(&missing, &memory),
                            structured,
                            Vec::new(),
                            0,
                        )
                    } else {
                        let score_records = match preloaded_score_records {
                            Some(score_records) => score_records,
                            None => {
                                self.query_scores_from_memory(&memory, score_query_year)
                                    .await?
                            }
                        };
                        let structured = build_probability_from_memory(&memory, &score_records);
                        let reply = render_probability_answer(&structured);
                        (
                            reply,
                            structured,
                            vec![ChatCitation {
                                year: None,
                                source_label: "哈尔滨师范大学历年录取统计表".to_owned(),
                                source_url: None,
                            }],
                            1,
                        )
                    }
                }
                RetrievalIntent::GeneralAnswer => {
                    if asks_province_admission_major_list(&input.message) {
                        self.handle_province_major_list_request(&input.message, &memory)
                            .await?
                    } else if asks_major_admission_province_list(&input.message) {
                        self.handle_major_province_list_request(&input.message)
                            .await?
                    } else {
                        let redirect = build_redirect_prompt(&memory);
                        let answer = "这是普通咨询问题，需要调用大模型生成自然回答。边界：录取线、录取概率、招生计划、招生政策、招生电话、官网链接、专业培养方案、学分和课程等事实，不能脱离工具证据编造；城市印象、校园生活体验、备考建议、沟通建议等低风险内容，可以用常识和模型能力给出亲切建议，并说明具体安排以学校官方通知为准。".to_owned();
                        let structured = ChatStructuredResult::GeneralAnswer {
                            answer: answer.clone(),
                            redirect_prompt: redirect.clone(),
                            collected_profile: memory.clone(),
                        };
                        (format!("{answer} {redirect}"), structured, Vec::new(), 0)
                    }
                }
            }
        };
        trace.push(domain::AgentTraceStep {
            step: 1,
            node: "retrieval_or_draft".to_owned(),
            tool_name: if tool_call_count > 0 {
                Some(match route_intent {
                    ChatIntent::ScoreQuery | ChatIntent::ProbabilityAssessment => {
                        "query_admission_scores".to_owned()
                    }
                    ChatIntent::KnowledgeAnswer => "search_knowledge".to_owned(),
                    _ => "tool".to_owned(),
                })
            } else {
                None
            },
            duration_ms: None,
            error: None,
        });

        Ok(PreparedChatTurn {
            started_at,
            conversation_id,
            user_message: input.message,
            memory,
            draft_reply: reply,
            structured_result,
            citations,
            route_intent,
            tool_call_count,
            trace,
            compressed,
        })
    }

    async fn finish_turn(
        &self,
        prepared: PreparedChatTurn,
        final_reply: String,
        model_call_count: usize,
        llm_model: Option<String>,
        synthesis_used: bool,
    ) -> Result<ChatReply> {
        let final_reply =
            finalize_reply(final_reply, &prepared.structured_result, &prepared.memory);

        let diagnostics = domain::ChatDiagnostics {
            mode: "custom_runtime".to_owned(),
            route_intent: Some(prepared.route_intent),
            total_duration_ms: prepared.started_at.elapsed().as_millis(),
            model_call_count,
            llm_model,
            synthesis_used,
            tool_call_count: prepared.tool_call_count,
            trace: prepared.trace,
            compression: Some(prepared.compressed.diagnostics),
        };

        self.db
            .append_message(
                &prepared.conversation_id,
                "assistant",
                &final_reply,
                Some(&prepared.structured_result),
                &prepared.citations,
            )
            .await?;

        Ok(ChatReply {
            conversation_id: prepared.conversation_id,
            reply: final_reply,
            structured_result: prepared.structured_result,
            citations: prepared.citations,
            diagnostics: Some(diagnostics),
        })
    }

    async fn conversation_turn_lock(&self, conversation_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.turn_locks.lock().await;
        if locks.len() > turn_lock_map_limit() {
            locks.retain(|key, lock| key == conversation_id || Arc::strong_count(lock) > 1);
        }
        locks
            .entry(conversation_id.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn query_scores_from_memory(
        &self,
        memory: &ResolvedMemory,
        year: Option<i32>,
    ) -> Result<ChatStructuredResult> {
        let major_slug = memory
            .major_slug
            .clone()
            .unwrap_or_else(|| memory.major_name.clone().unwrap_or_default());
        let major_name = memory
            .major_name
            .clone()
            .unwrap_or_else(|| major_slug.clone());
        self.retrieval
            .query_scores(
                memory
                    .province_name
                    .as_deref()
                    .or(memory.province_code.as_deref())
                    .unwrap(),
                &major_slug,
                &major_name,
                memory.subject_type.as_deref(),
                year,
            )
            .await
    }

    async fn handle_combined_request(
        &self,
        message: &str,
        memory: &ResolvedMemory,
        plan: CombinedRequestPlan,
        score_query_year: Option<i32>,
    ) -> Result<(String, ChatStructuredResult, Vec<ChatCitation>, usize)> {
        let mut results = Vec::new();
        let mut citations = Vec::new();
        let mut tool_count = 0usize;
        let mut score_had_records = false;

        if asks_score_comparison(message) {
            if memory.province_name.is_none() && memory.province_code.is_none() {
                let missing = vec!["province".to_owned()];
                let structured = ChatStructuredResult::FollowUp {
                    pending_intent: ChatIntent::ScoreQuery,
                    missing_fields: missing.clone(),
                    collected_profile: memory.clone(),
                };
                return Ok((
                    render_follow_up(&missing, memory),
                    structured,
                    Vec::new(),
                    0,
                ));
            }
            let candidates = self.retrieval.search_major_candidates(message, 8).await?;
            let distinct = resolve_score_comparison_candidates(message, memory, candidates);
            if distinct.len() >= 2 {
                let province = memory
                    .province_name
                    .as_deref()
                    .or(memory.province_code.as_deref())
                    .unwrap()
                    .to_owned();
                let subject_type = memory.subject_type.clone();
                let first = distinct[0].clone();
                let second = distinct[1].clone();
                let (first_result, second_result) = tokio::try_join!(
                    self.retrieval.query_scores(
                        &province,
                        &first.slug,
                        &first.name,
                        subject_type.as_deref(),
                        score_query_year,
                    ),
                    self.retrieval.query_scores(
                        &province,
                        &second.slug,
                        &second.name,
                        subject_type.as_deref(),
                        score_query_year,
                    )
                )?;
                citations.extend(citations_from_structured_result(&first_result));
                citations.extend(citations_from_structured_result(&second_result));
                results.push(first_result);
                results.push(second_result);
                tool_count += 2;
            }
        }

        if (plan.include_score || plan.include_probability) && !asks_score_comparison(message) {
            let missing = if plan.include_probability {
                effective_probability_missing_fields(message, memory)
            } else {
                missing_score_fields(memory)
            };
            if !missing.is_empty() {
                let pending_intent = if plan.include_probability {
                    ChatIntent::ProbabilityAssessment
                } else {
                    ChatIntent::ScoreQuery
                };
                let structured = ChatStructuredResult::FollowUp {
                    pending_intent,
                    missing_fields: missing.clone(),
                    collected_profile: memory.clone(),
                };
                return Ok((
                    render_follow_up(&missing, memory),
                    structured,
                    Vec::new(),
                    0,
                ));
            }

            let score_result = self
                .query_scores_from_memory(memory, score_query_year)
                .await?;
            score_had_records = score_result_has_records(&score_result);
            citations.extend(citations_from_structured_result(&score_result));
            tool_count += 1;

            if plan.include_probability {
                let probability = build_probability_from_memory(memory, &score_result);
                results.push(probability);
            }
            if plan.include_score {
                results.push(score_result);
            }
        }

        if plan.include_knowledge && (!plan.knowledge_when_score_empty || !score_had_records) {
            let knowledge_query = contextual_knowledge_query(message, memory);
            let knowledge = self
                .retrieve_knowledge_with_memory_focus(&knowledge_query, memory)
                .await?;
            citations.extend(knowledge.citations);
            results.push(knowledge.structured_result);
            tool_count += 1;
        }

        let structured = compact_evidence_bundle(message, results);
        let citations = dedupe_citations(citations);
        let reply = render_evidence_bundle_answer(&structured);
        Ok((reply, structured, citations, tool_count))
    }

    async fn handle_province_major_list_request(
        &self,
        message: &str,
        memory: &ResolvedMemory,
    ) -> Result<(String, ChatStructuredResult, Vec<ChatCitation>, usize)> {
        let province = extract_known_province(message)
            .or_else(|| memory.province_name.clone())
            .or_else(|| memory.province_code.clone())
            .unwrap_or_default();
        let subject_type = extract_subject_type(message);
        let structured = self
            .retrieval
            .list_province_admission_majors(
                &province,
                subject_type.as_deref(),
                extract_year_from_message(message),
            )
            .await?;
        let citations = citations_from_structured_result(&structured);
        let reply = render_province_major_list_answer_for_query(&structured, message);
        Ok((reply, structured, citations, 1))
    }

    async fn handle_major_province_list_request(
        &self,
        message: &str,
    ) -> Result<(String, ChatStructuredResult, Vec<ChatCitation>, usize)> {
        let candidates = self.retrieval.search_major_candidates(message, 4).await?;
        let distinct = distinct_major_candidates(candidates, 2);
        let Some(candidate) = select_unambiguous_major_candidate(message, &distinct) else {
            if distinct.len() > 1 {
                let structured =
                    render_major_disambiguation(message, distinct, ChatIntent::KnowledgeAnswer);
                let reply = render_major_disambiguation_reply(&structured);
                return Ok((reply, structured, Vec::new(), 1));
            }
            let structured =
                render_major_disambiguation(message, Vec::new(), ChatIntent::KnowledgeAnswer);
            let reply = render_major_disambiguation_reply(&structured);
            return Ok((reply, structured, Vec::new(), 1));
        };

        let subject_type = extract_subject_type(message);
        let structured = self
            .retrieval
            .list_major_admission_provinces(
                &candidate.slug,
                &candidate.name,
                subject_type.as_deref(),
                extract_year_from_message(message),
            )
            .await?;
        let citations = citations_from_structured_result(&structured);
        let reply = render_major_province_list_answer(&structured);
        Ok((reply, structured, citations, 1))
    }

    async fn retrieve_knowledge_with_memory_focus(
        &self,
        query: &str,
        memory: &ResolvedMemory,
    ) -> Result<retrieval::KnowledgeRetrievalResult> {
        if should_force_training_plan_major_focus(query, memory)
            || Self::should_force_training_plan_query_major_focus(query)
        {
            if let Some(major) = extract_major_phrase(query)
                .filter(|_| !comparison_uses_contextual_major(query))
                .or_else(|| explicit_major_text(query))
            {
                return self
                    .retrieval
                    .retrieve_knowledge_for_major(query, &major)
                    .await;
            }
            if let Some(major) = memory.major_name.as_deref() {
                return self
                    .retrieval
                    .retrieve_knowledge_for_major(query, major)
                    .await;
            }
        }
        self.retrieval.retrieve_knowledge(query).await
    }

    fn should_force_training_plan_query_major_focus(query: &str) -> bool {
        !is_admission_policy_query(query)
            && !asks_program_comparison_context(query)
            && asks_training_plan_context(query)
            && extract_major_phrase(query).is_some()
    }

    async fn resolve_major_from_message(
        &self,
        memory: &mut ResolvedMemory,
        message: &str,
    ) -> Result<()> {
        if asks_major_group_without_college(message)
            || contains_policy_program_term(message)
            || asks_province_admission_major_list(message)
            || asks_score_comparison(message) && comparison_uses_contextual_major(message)
        {
            return Ok(());
        }
        let query = extract_switch_major_query(message).unwrap_or_else(|| message.to_owned());
        let mut candidates = self.retrieval.search_major_candidates(&query, 3).await?;
        let mut used_memory_fallback = false;
        if candidates.is_empty() {
            let Some(query) = memory
                .major_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            else {
                return Ok(());
            };
            candidates = self.retrieval.search_major_candidates(query, 3).await?;
            used_memory_fallback = true;
        }
        let Some(candidate) = candidates.first() else {
            return Ok(());
        };
        if memory.major_name.is_none()
            || message.contains(&candidate.name)
            || major_alias_matches(message, &candidate.name)
            || is_major_switch_message(message)
            || memory.major_name.as_deref().is_some_and(|value| {
                used_memory_fallback && major_alias_matches(value, &candidate.name)
            })
        {
            memory.major_slug = Some(candidate.slug.clone());
            memory.major_name = Some(candidate.name.clone());
        }
        Ok(())
    }

    async fn synthesize_reply(
        &self,
        user_message: &str,
        draft_reply: &str,
        structured_result: &ChatStructuredResult,
        citations: &[ChatCitation],
        memory: &ResolvedMemory,
        compressed: &agent_runtime::CompressedContext,
    ) -> Result<Option<String>> {
        let Some(llm) = &self.llm else {
            return Ok(None);
        };
        let messages = self.synthesis_messages(
            user_message,
            draft_reply,
            structured_result,
            citations,
            memory,
            compressed,
        )?;
        let response = llm.complete(&messages).await?;
        let content = response.content.trim().to_owned();
        if content.is_empty() {
            Ok(None)
        } else {
            Ok(Some(content))
        }
    }

    fn synthesis_messages(
        &self,
        user_message: &str,
        draft_reply: &str,
        structured_result: &ChatStructuredResult,
        citations: &[ChatCitation],
        memory: &ResolvedMemory,
        compressed: &agent_runtime::CompressedContext,
    ) -> Result<Vec<LlmMessage>> {
        let structured_json = serde_json::to_string(structured_result)?;
        let citations_json = serde_json::to_string(citations)?;
        let turn_context_json = build_synthesis_turn_context(memory, compressed)?;
        let turn_constraint = turn_synthesis_constraint(user_message, structured_result);
        let response_shape = response_shape_instruction(user_message, structured_result);
        Ok(vec![
            LlmMessage {
                    role: MessageRole::System,
                    content:
                        "你是哈尔滨师范大学招生智能顾问，回答面向学生和家长，要自然、亲切、准确，不要说“知识库命中”“有资料”“字段”等后台词。\n\n优先级规则：\n1. 系统边界最高，其次是用户本轮问题。必须先回答用户本轮真正想问的内容，不要因为模板或历史上下文转移主题。\n2. 结构化结果、证据和引用是事实来源；草稿回答只是提示，不可覆盖用户意图。\n3. 对话上下文、confirmed memory、active referents 和压缩摘要只用于理解“它、这个专业、刚才那个、继续”等多轮指代，不是事实证据；如果它们与本轮结构化结果或证据冲突，必须以本轮结构化结果和证据为准。\n\n事实边界分层：\n1. 高风险事实必须严格依据给定结构化结果、证据和引用回答，包括录取线、位次、录取概率、招生计划、招生政策、招生电话、官网链接、专业目录、培养方案、课程、学分、毕业要求、体检、语种、调剂、同分规则、校训、学校章程、办学定位和学校官方历史沿革。没有证据时不要编造，要说明还需要按招生简章、培养方案、FAQ、官网或录取统计核对。\n2. 低风险泛聊可以发挥模型常识，包括城市印象、大学生活建议、备考建议、如何和家长沟通、入学前准备等；涉及学校具体安排时要用“通常/一般/建议以学校通知为准”等边界表达。\n3. 如果已经给出结构化证据，要优先用证据，不要把培养方案覆盖专业说成当年一定招生，不要把相近专业数据说成目标专业数据。培养方案问题只允许使用目标专业的培养方案证据；如果证据来自其他专业，要明确不能据此回答目标专业。\n4. 招生联系方式只能使用证据中出现的号码和网址；不要补写学院电话或自行推测工作时间。\n5. 艺术类录取规则只能按证据表述。没有证据时，不得自行生成“文化课×40%+专业课×60%”等折算公式。\n6. 概率和普通高考录取线解释不得混用专升本、单招、预科等非普通高考批次；如果证据批次不可比，要说明边界。\n7. 普通闲聊、身份介绍、能力介绍也要根据用户原话直接回应，不要套用无关模板；用户问“你擅长什么”时，回答能力范围即可，不要要求用户补省份分数。\n8. 用户用“和、或、以及、哪些”等方式同时问多个事实点时，要逐项回应；证据只覆盖其中一部分时，先回答有证据的部分，再明确说明其余部分在当前证据中没有直接条款支持，不能自行补充。\n9. 只有当用户正在问录取概率、分数线、志愿建议或明确需要个性化判断时，才追问省份、科类、分数、位次、意向专业；用户只是问学校介绍、校园生活或能力介绍时，不要用画像追问收尾。"
                            .to_owned(),
            },
            LlmMessage {
                    role: MessageRole::User,
                    content: format!(
                        "用户问题：{user_message}\n\n对话上下文（仅用于指代理解，不作为事实证据）：{}\n\n结构化结果：{}\n\n引用：{}\n\n草稿回答：{draft_reply}{turn_constraint}\n\n请输出最终中文回答，{response_shape}，保留关键省份、科类、分数、专业、年份和证据边界。",
                        truncate_chars(&turn_context_json, 3000),
                        truncate_chars(&structured_json, 6000),
                        truncate_chars(&citations_json, 1800),
                    ),
            },
        ])
    }
}

const SYNTHESIS_RECENT_MESSAGE_LIMIT: usize = 6;

fn response_shape_instruction(
    user_message: &str,
    structured_result: &ChatStructuredResult,
) -> &'static str {
    if matches!(structured_result, ChatStructuredResult::Greeting { .. }) {
        return "1-2个短段落，总字数控制在120字以内";
    }
    if is_broad_school_or_campus_query(user_message) {
        return "2-3个自然段，先讲学校整体，再补充特色和资料边界";
    }
    "2-5段"
}

fn build_synthesis_turn_context(
    memory: &ResolvedMemory,
    compressed: &agent_runtime::CompressedContext,
) -> Result<String> {
    let recent_messages = compressed
        .messages
        .iter()
        .rev()
        .take(SYNTHESIS_RECENT_MESSAGE_LIMIT)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|message| {
            json!({
                "role": message.role,
                "content": truncate_chars(&message.content, 360),
                "structuredType": message
                    .structured_payload
                    .as_ref()
                    .map(ChatStructuredResult::kind)
            })
        })
        .collect::<Vec<_>>();

    let mut recent_user_major_mentions = Vec::new();
    let mut seen_major_mentions = HashSet::new();
    for message in compressed.messages.iter().rev() {
        if message.role != "user"
            || is_admission_policy_query(&message.content)
            || !should_update_major_from_knowledge_query(&message.content)
            || !message_explicitly_names_major_for_memory(&message.content)
        {
            continue;
        }
        if let Some(major) = extract_major_phrase(&message.content) {
            if seen_major_mentions.insert(major.clone()) {
                recent_user_major_mentions.push(major);
            }
        }
        if recent_user_major_mentions.len() >= 5 {
            break;
        }
    }

    let active_referents = json!({
        "currentProvince": memory.province_name.as_ref().or(memory.province_code.as_ref()),
        "currentSubjectType": memory.subject_type,
        "currentScore": memory.score,
        "currentRank": memory.rank,
        "currentMajor": memory.major_name.as_ref().or(memory.major_slug.as_ref()),
        "intendedMajors": memory.intended_majors,
        "pendingIntent": memory.pending_intent,
        "recentUserMajorMentions": recent_user_major_mentions
    });

    let context = json!({
        "confirmedMemory": memory,
        "activeReferents": active_referents,
        "recentMessages": recent_messages,
        "compressedSummary": compressed.summary,
        "compression": compressed.diagnostics,
        "usageRule": "Use this only to resolve multi-turn references. Do not use it as factual evidence when structured results or citations are missing."
    });
    Ok(serde_json::to_string(&context)?)
}

fn should_avoid_profile_follow_up(
    user_message: &str,
    structured_result: &ChatStructuredResult,
) -> bool {
    if matches!(
        structured_result,
        ChatStructuredResult::ScoreQuery { .. }
            | ChatStructuredResult::ProbabilityAssessment { .. }
            | ChatStructuredResult::FollowUp { .. }
    ) {
        return false;
    }
    !asks_score_line(user_message)
        && !asks_probability(user_message)
        && extract_score(user_message).is_none()
}

fn turn_synthesis_constraint(
    user_message: &str,
    structured_result: &ChatStructuredResult,
) -> String {
    let mut constraints = Vec::new();
    if should_avoid_profile_follow_up(user_message, structured_result) {
        constraints.push("用户不是在请求录取概率、分数线或志愿个性化评估。请只回答本轮问题，不要在结尾主动要求用户提供省份、科类、分数、位次或意向专业。");
    }
    if matches!(structured_result, ChatStructuredResult::Greeting { .. }) {
        constraints.push("用户在询问身份或能力范围。最终回答控制在1-2个短段落，直接说明能做什么即可，不要展开长示例、不要过度拟人化。");
    }
    if is_broad_school_or_campus_query(user_message) {
        constraints.push("用户在询问学校整体介绍。不要继承上一轮具体专业作为回答重点；可以概括学校定位、师范特色、学科门类、校区和校园生活，但不要把某个专业培养方案当成学校介绍主体。");
    }
    if is_compound_fact_question(user_message) {
        constraints.push("用户本轮同时问了多个事实点。最终回答必须逐项回应每个事实点；如果结构化证据只覆盖其中一部分，要明确说明未覆盖部分在当前证据中没有直接条款支持，不能把相近条款当成确定事实。");
    }
    let policy_facets = policy_facets_in_message(user_message);
    let policy_constraint = if !policy_facets.is_empty()
        && matches!(
            structured_result,
            ChatStructuredResult::KnowledgeAnswer { .. }
        ) {
        Some(format!(
            "用户本轮明确问到这些政策点：{}。最终回答必须逐项覆盖这些政策点，并保留这些原词或清晰同义词；证据没有直接覆盖的政策点，也要明确说明当前证据中未见直接条款支持。",
            policy_facets.join("、")
        ))
    } else {
        None
    };
    if let Some(policy_constraint) = &policy_constraint {
        constraints.push(policy_constraint.as_str());
    }
    if probability_uses_unspecified_subject_records(structured_result).is_some() {
        constraints.push("结构化结果的实际录取记录科类是“未区分”。如果用户提到了历史类、物理类、文科或理科，必须说明统计表未单列该科类，只能按未区分科类/普通类记录作参考；不得说暂未找到该专业录取记录。");
    }
    if score_query_uses_unspecified_subject_records(structured_result).is_some() {
        constraints.push("结构化分数线结果的实际录取记录科类是“未区分”。如果用户提到了历史类、物理类、文科或理科，必须说明统计表未单列该科类，只能按未区分科类/普通类记录作参考；不得把这些记录说成该指定科类的单列录取线。");
    }
    if constraints.is_empty() {
        String::new()
    } else {
        format!("\n\n本轮任务约束：{}", constraints.join(" "))
    }
}

fn is_compound_fact_question(message: &str) -> bool {
    contains_any_text(message, &["或", "和", "以及", "哪些"])
        && contains_any_text(
            message,
            &[
                "限制", "要求", "规则", "安排", "课程", "学分", "实践", "实习",
            ],
        )
}

fn policy_facets_in_message(message: &str) -> Vec<&'static str> {
    let candidates = [
        ("外语语种", &["外语语种", "外语", "语种"][..]),
        ("单科成绩", &["单科成绩", "单科", "英语成绩"][..]),
        ("专业级差", &["专业级差", "级差"][..]),
        ("服从调剂", &["服从调剂", "调剂"][..]),
        ("退档", &["退档"][..]),
        ("同分排序", &["同分", "分数相同", "成绩相同", "排序"][..]),
        ("体检要求", &["体检", "身体健康"][..]),
        ("选考科目", &["选考", "选科", "科目要求"][..]),
        ("男女比例", &["男女比例", "性别比例", "男生", "女生"][..]),
    ];
    candidates
        .iter()
        .filter_map(|(facet, markers)| contains_any_text(message, markers).then_some(*facet))
        .collect()
}

pub fn chunk_reply_text(reply: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in reply.chars() {
        current.push(ch);
        if matches!(ch, '。' | '！' | '？' | '\n') || current.chars().count() >= 24 {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn conversation_history_window() -> i64 {
    std::env::var("CONVERSATION_HISTORY_WINDOW")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(40)
}

fn turn_lock_map_limit() -> usize {
    std::env::var("CONVERSATION_TURN_LOCK_MAP_LIMIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value >= 128)
        .unwrap_or(4096)
}

#[allow(dead_code)]
pub fn build_runtime_context(
    conversation_id: String,
    user_message: String,
    memory: ResolvedMemory,
    history: Vec<ConversationMessage>,
) -> RuntimeContext {
    RuntimeContext {
        conversation_id,
        user_message,
        memory,
        history,
        route_intent: None,
        structured_result: None,
        draft_reply: None,
        compression: None,
    }
}

#[allow(dead_code)]
pub fn finish_runtime_context(context: RuntimeContext) -> RuntimeOutput {
    RuntimeOutput {
        context,
        diagnostics: domain::ChatDiagnostics {
            mode: "custom_runtime".to_owned(),
            route_intent: None,
            total_duration_ms: 0,
            model_call_count: 0,
            llm_model: None,
            synthesis_used: false,
            tool_call_count: 0,
            trace: Vec::new(),
            compression: None,
        },
    }
}

fn to_chat_intent(intent: &RetrievalIntent) -> ChatIntent {
    match intent {
        RetrievalIntent::Greeting => ChatIntent::Greeting,
        RetrievalIntent::ScoreQuery => ChatIntent::ScoreQuery,
        RetrievalIntent::ProbabilityAssessment => ChatIntent::ProbabilityAssessment,
        RetrievalIntent::KnowledgeAnswer => ChatIntent::KnowledgeAnswer,
        RetrievalIntent::GeneralAnswer => ChatIntent::GeneralAnswer,
    }
}

fn latest_assistant_structured(history: &[ConversationMessage]) -> Option<&ChatStructuredResult> {
    history
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .and_then(|message| message.structured_payload.as_ref())
}

#[derive(Debug, Clone, Copy)]
struct CombinedRequestPlan {
    include_score: bool,
    include_probability: bool,
    include_knowledge: bool,
    knowledge_when_score_empty: bool,
}

fn combined_request_plan(
    message: &str,
    memory: &ResolvedMemory,
    route_intent: &RetrievalIntent,
    last_assistant_structured: Option<&ChatStructuredResult>,
) -> Option<CombinedRequestPlan> {
    if matches!(
        route_intent,
        RetrievalIntent::Greeting | RetrievalIntent::GeneralAnswer
    ) {
        return None;
    }

    let asks_score = asks_score_line(message) || asks_score_comparison(message);
    let asks_probability =
        asks_probability(message) || matches!(route_intent, RetrievalIntent::ProbabilityAssessment);
    let include_score_result = asks_score
        && (!asks_probability
            || asks_explicit_score_result_with_probability(message)
            || asks_score_comparison(message));
    let knowledge_when_score_empty = asks_score_with_likely_training_plan_followup(message)
        && !needs_parallel_knowledge_for_score_line(message);
    let asks_knowledge = asks_training_plan_context(message)
        || matches!(route_intent, RetrievalIntent::KnowledgeAnswer)
            && (asks_score || asks_probability)
        || knowledge_when_score_empty
        || needs_parallel_knowledge_for_score_line(message);
    let asks_multi_major_score = asks_score_comparison(message);
    let continues_score_probability_bundle =
        last_was_score_probability_bundle(last_assistant_structured)
            && has_score_context(memory)
            && memory.score.is_some()
            && (is_short_follow_up(message)
                || is_major_switch_message(message)
                || extract_known_province(message).is_some()
                || extract_score(message).is_some());
    let explicit_combo = (include_score_result && asks_probability)
        || (include_score_result && asks_knowledge)
        || (asks_probability && asks_knowledge)
        || asks_multi_major_score
        || continues_score_probability_bundle;

    if !explicit_combo {
        return None;
    }

    let has_major_context = memory.major_name.is_some() || memory.major_slug.is_some();
    let has_province_context = memory.province_name.is_some() || memory.province_code.is_some();
    if !has_major_context && !asks_multi_major_score {
        return None;
    }
    if !has_province_context && (asks_score || asks_probability) {
        return None;
    }

    Some(CombinedRequestPlan {
        include_score: include_score_result
            || asks_multi_major_score
            || continues_score_probability_bundle,
        include_probability: asks_probability || continues_score_probability_bundle,
        include_knowledge: asks_knowledge,
        knowledge_when_score_empty,
    })
}

fn last_was_score_probability_bundle(result: Option<&ChatStructuredResult>) -> bool {
    let Some(ChatStructuredResult::EvidenceBundle { results, .. }) = result else {
        return false;
    };
    let has_score = results
        .iter()
        .any(|item| matches!(item, ChatStructuredResult::ScoreQuery { .. }));
    let has_probability = results
        .iter()
        .any(|item| matches!(item, ChatStructuredResult::ProbabilityAssessment { .. }));
    has_score && has_probability
}

fn asks_score_line(message: &str) -> bool {
    [
        "录取线",
        "分数线",
        "最低分",
        "最低位次",
        "投档线",
        "投档分",
        "录取分",
        "录取情况",
        "录取数据",
        "录取统计",
        "历年分",
        "近三年",
        "近五年",
        "2021到2025",
        "2021年到2025年",
        "2021-2025",
        "2021年-2025年",
        "2021—2025",
        "2021年—2025年",
        "2021至2025",
        "2021年至2025年",
        "21到25",
        "21-25",
        "多少分",
        "几分",
        "多少位次",
    ]
    .iter()
    .any(|item| message.contains(item))
}

fn asks_explicit_score_result_with_probability(message: &str) -> bool {
    asks_probability(message)
        && contains_any_text(
            message,
            &[
                "录取线",
                "分数线",
                "最低分",
                "最低位次",
                "录取分",
                "列一下",
                "列出",
                "是多少",
                "分别是多少",
                "多少分",
                "几分",
            ],
        )
}

fn asks_probability(message: &str) -> bool {
    [
        "概率",
        "能上",
        "能不能上",
        "能报",
        "能录取",
        "能不能录取",
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
    ]
    .iter()
    .any(|item| message.contains(item))
}

fn asks_score_comparison(message: &str) -> bool {
    (message.contains("哪个") || message.contains("谁") || message.contains("比较"))
        && (message.contains("分数") || message.contains("录取线") || message.contains("更高"))
}

fn asks_training_plan_context(message: &str) -> bool {
    [
        "培养方案",
        "课程",
        "培养目标",
        "毕业要求",
        "毕业条件",
        "实践环节",
        "教育实习",
        "第二课堂",
        "创新实践",
    ]
    .iter()
    .any(|item| message.contains(item))
        || asks_major_fit_context(message)
        || asks_program_comparison_context(message)
}

fn asks_major_fit_context(message: &str) -> bool {
    contains_any_text(message, &["适合", "合适", "匹配", "契合", "适不适合"])
        && contains_any_text(
            message,
            &["专业", "培养", "老师", "教师", "实验", "课程", "就业"],
        )
}

fn asks_program_comparison_context(message: &str) -> bool {
    contains_any_text(
        message,
        &[
            "区别",
            "差别",
            "差异",
            "不同",
            "相比",
            "比较",
            "对比",
            "哪个更适合",
            "哪个更好",
            "怎么选",
        ],
    ) && contains_any_text(
        message,
        &[
            "专业",
            "学院",
            "班",
            "师范",
            "非师范",
            "行知",
            "实验班",
            "培养",
            "课程",
            "学位",
            "实践",
            "就业",
            "升学",
        ],
    )
}

fn asks_score_with_likely_training_plan_followup(message: &str) -> bool {
    asks_score_line(message)
        && !asks_probability(message)
        && (message.contains("2021到2025") || message.contains("21到25"))
}

fn needs_parallel_knowledge_for_score_line(message: &str) -> bool {
    asks_score_line(message)
        && contains_any_text(message, &["2021到2025", "21到25", "近三年", "历年"])
        && contains_any_text(
            message,
            &[
                "艺术类",
                "综合分",
                "专业课",
                "美术",
                "绘画",
                "书法",
                "设计",
                "视觉传达",
                "环境设计",
                "音乐",
                "舞蹈",
                "作曲",
                "表演",
                "播音",
                "体育",
            ],
        )
}

fn build_probability_from_memory(
    memory: &ResolvedMemory,
    score_records: &ChatStructuredResult,
) -> ChatStructuredResult {
    let major_name = memory
        .major_name
        .as_deref()
        .or(memory.major_slug.as_deref())
        .unwrap_or("目标专业");
    let (score_summary, score_history) = match score_records {
        ChatStructuredResult::ScoreQuery {
            records, summary, ..
        } => {
            let probability_records = records
                .iter()
                .filter(|record| is_general_admission_score_record(record))
                .collect::<Vec<_>>();
            (
                json!({
                    "recordCount": probability_records.len(),
                    "years": summary.years,
                    "latestMinScore": probability_records.first().map(|record| record.min_score),
                    "latestYear": probability_records.first().map(|record| record.year),
                    "records": probability_records.iter().take(5).collect::<Vec<_>>()
                }),
                probability_records
                    .iter()
                    .map(|record| ProbabilityScoreHistoryItem {
                        year: record.year,
                        min_score: record.min_score,
                        min_rank: record.min_rank,
                    })
                    .collect::<Vec<_>>(),
            )
        }
        _ => (json!({}), Vec::new()),
    };
    let subject_type = probability_subject_type_from_records(memory, score_records);
    let requested_subject_type = memory
        .subject_type
        .as_deref()
        .filter(|requested| {
            subject_type
                .as_deref()
                .is_some_and(|actual| actual != *requested)
        })
        .map(ToOwned::to_owned);
    let plan_history: Vec<ProbabilityPlanHistoryItem> = Vec::new();
    let engine = calculate_admission_probability(ProbabilityEngineInput {
        score: memory.score.unwrap_or(0.0),
        rank: memory.rank,
        score_history,
        plan_history: plan_history.clone(),
        source_mode: ProbabilitySourceMode::Major,
    });
    ChatStructuredResult::ProbabilityAssessment {
        assessment: json!({
            "province": memory.province_name.as_deref().or(memory.province_code.as_deref()),
            "subjectType": subject_type,
            "requestedSubjectType": requested_subject_type,
            "score": memory.score,
            "rank": memory.rank,
            "major": major_name,
            "probability": engine.probability,
            "level": engine.level.as_str(),
            "confidence": engine.confidence.as_str(),
            "summary": engine.summary,
            "factors": engine.factors,
            "disclaimer": engine.disclaimer,
            "basis": {
                "scoreDataMode": "major",
                "scoreYearsUsed": score_summary
                    .get("recordCount")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0),
                "planYearsUsed": plan_history.len()
            },
            "metrics": engine.metrics,
            "scoreSummary": score_summary,
            "message": "已使用确定性概率引擎，基于历年录取分数、位次和数据完整度进行参考评估。"
        }),
    }
}

fn probability_subject_type_from_records(
    memory: &ResolvedMemory,
    score_records: &ChatStructuredResult,
) -> Option<String> {
    let requested = memory.subject_type.clone();
    let ChatStructuredResult::ScoreQuery { records, .. } = score_records else {
        return requested;
    };
    if records.is_empty() {
        return requested;
    }
    if let Some(requested) = requested.as_deref() {
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
    requested
}

fn compact_evidence_bundle(
    message: &str,
    results: Vec<ChatStructuredResult>,
) -> ChatStructuredResult {
    let mut seen = HashSet::new();
    let mut compacted = Vec::new();
    for result in results {
        let key = evidence_key(&result);
        if seen.insert(key) {
            compacted.push(result);
        }
    }
    if compacted.len() == 1 {
        return compacted.into_iter().next().unwrap();
    }
    ChatStructuredResult::EvidenceBundle {
        message: message.to_owned(),
        results: compacted,
    }
}

fn score_result_has_records(result: &ChatStructuredResult) -> bool {
    matches!(result, ChatStructuredResult::ScoreQuery { records, .. } if !records.is_empty())
}

fn is_general_admission_score_record(record: &domain::AdmissionScoreRecord) -> bool {
    !contains_any_text(&record.batch, &["专升本", "单招", "预科"])
}

fn evidence_key(result: &ChatStructuredResult) -> String {
    match result {
        ChatStructuredResult::ScoreQuery {
            major_name,
            province,
            subject_type,
            ..
        } => format!(
            "score:{province}:{major_name}:{}",
            subject_type.as_deref().unwrap_or("")
        ),
        ChatStructuredResult::ProbabilityAssessment { assessment } => format!(
            "probability:{}:{}:{}:{}",
            assessment
                .get("province")
                .and_then(|value| value.as_str())
                .unwrap_or(""),
            assessment
                .get("subjectType")
                .and_then(|value| value.as_str())
                .unwrap_or(""),
            assessment
                .get("score")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0),
            assessment
                .get("major")
                .and_then(|value| value.as_str())
                .unwrap_or("")
        ),
        ChatStructuredResult::KnowledgeAnswer { query, .. } => format!("knowledge:{query}"),
        ChatStructuredResult::ProvinceMajorList {
            province,
            subject_type,
            year,
            ..
        } => format!(
            "province-major-list:{province}:{}:{}",
            subject_type.as_deref().unwrap_or(""),
            year.map(|value| value.to_string()).unwrap_or_default()
        ),
        ChatStructuredResult::MajorProvinceList {
            major_name,
            subject_type,
            year,
            ..
        } => format!(
            "major-province-list:{major_name}:{}:{}",
            subject_type.as_deref().unwrap_or(""),
            year.map(|value| value.to_string()).unwrap_or_default()
        ),
        other => other.kind().to_owned(),
    }
}

fn dedupe_citations(citations: Vec<ChatCitation>) -> Vec<ChatCitation> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for citation in citations {
        let key = format!(
            "{}:{}:{}",
            citation
                .year
                .map(|value| value.to_string())
                .unwrap_or_default(),
            citation.source_label,
            citation.source_url.as_deref().unwrap_or_default()
        );
        if seen.insert(key) {
            deduped.push(citation);
        }
    }
    deduped
}

fn distinct_major_candidates(
    candidates: Vec<domain::MajorCandidate>,
    limit: usize,
) -> Vec<domain::MajorCandidate> {
    let mut seen_roots = HashSet::new();
    let mut distinct = Vec::new();
    for candidate in candidates {
        let key = normalize_major_alias(&candidate.name);
        if seen_roots.insert(key) {
            distinct.push(candidate);
        }
        if distinct.len() >= limit {
            break;
        }
    }
    distinct
}

fn resolve_score_comparison_candidates(
    message: &str,
    memory: &ResolvedMemory,
    candidates: Vec<domain::MajorCandidate>,
) -> Vec<domain::MajorCandidate> {
    let mut selected = Vec::new();
    let uses_contextual_major = comparison_uses_contextual_major(message);
    if uses_contextual_major {
        if let Some(current) = memory_major_candidate(memory) {
            selected.push(current);
        }
    }

    let explicit = candidates
        .iter()
        .filter(|candidate| major_alias_matches(message, &candidate.name))
        .cloned()
        .collect::<Vec<_>>();
    let pool = if explicit.is_empty() {
        candidates
    } else {
        explicit
    };

    for candidate in pool {
        if selected.iter().any(|existing: &domain::MajorCandidate| {
            major_alias_matches(&existing.name, &candidate.name)
        }) {
            continue;
        }
        selected.push(candidate);
        if selected.len() >= 2 {
            break;
        }
    }

    distinct_major_candidates(selected, 2)
}

fn comparison_uses_contextual_major(message: &str) -> bool {
    contains_any_text(
        message,
        &["它", "这个专业", "该专业", "刚才", "之前", "上面", "前面"],
    ) || message.trim_start().starts_with("那")
}

fn memory_major_candidate(memory: &ResolvedMemory) -> Option<domain::MajorCandidate> {
    let name = memory
        .major_name
        .as_ref()
        .or(memory.major_slug.as_ref())?
        .trim();
    if name.is_empty() {
        return None;
    }
    Some(domain::MajorCandidate {
        slug: memory.major_slug.clone().unwrap_or_else(|| name.to_owned()),
        name: name.to_owned(),
        code: None,
        is_normal_major: name.contains("师范") && !name.contains("非师范"),
        latest_score: None,
    })
}

fn select_unambiguous_major_candidate<'a>(
    message: &str,
    candidates: &'a [domain::MajorCandidate],
) -> Option<&'a domain::MajorCandidate> {
    let first = candidates.first()?;
    let matching = candidates
        .iter()
        .filter(|candidate| major_alias_matches(message, &candidate.name))
        .collect::<Vec<_>>();
    if matching.len() == 1 {
        return matching.first().copied();
    }

    if !major_alias_matches(message, &first.name) {
        return (candidates.len() == 1).then_some(first);
    }

    if !is_policy_variant_major(&first.name)
        && matching
            .iter()
            .skip(1)
            .all(|candidate| is_policy_variant_major(&candidate.name))
    {
        return Some(first);
    }

    (candidates.len() == 1).then_some(first)
}

fn is_policy_variant_major(name: &str) -> bool {
    contains_any_text(
        name,
        &[
            "固边",
            "公费",
            "优师",
            "专项",
            "定向",
            "省属",
            "地方",
            "少数民族",
            "实验班",
            "中美",
            "121",
        ],
    )
}

fn has_knowledge_evidence(result: &ChatStructuredResult) -> bool {
    matches!(
        result,
        ChatStructuredResult::KnowledgeAnswer {
            faq,
            policies,
            vector_chunks,
            ..
        } if !faq.is_empty() || !policies.is_empty() || !vector_chunks.is_empty()
    )
}

fn backfill_contextual_training_plan_chunks(
    result: &mut ChatStructuredResult,
    history: &[ConversationMessage],
) {
    let ChatStructuredResult::KnowledgeAnswer {
        query,
        vector_chunks,
        ..
    } = result
    else {
        return;
    };
    if !vector_chunks.is_empty()
        || is_admission_policy_query(query)
        || !asks_training_plan_context(query)
    {
        return;
    }

    let query_major = extract_major_phrase(query);
    for message in history.iter().rev() {
        let Some(ChatStructuredResult::KnowledgeAnswer {
            vector_chunks: previous_chunks,
            ..
        }) = message.structured_payload.as_ref()
        else {
            continue;
        };
        if previous_chunks.is_empty() {
            continue;
        }
        let matching_chunks = previous_chunks
            .iter()
            .filter(|chunk| {
                query_major.as_deref().is_none_or(|major| {
                    chunk
                        .metadata
                        .get("majorName")
                        .and_then(|value| value.as_str())
                        .is_none_or(|chunk_major| major_alias_matches(major, chunk_major))
                })
            })
            .take(4)
            .cloned()
            .collect::<Vec<_>>();
        if !matching_chunks.is_empty() {
            *vector_chunks = matching_chunks;
            return;
        }
    }
}

fn score_query_uses_only_unspecified_subject(result: &ChatStructuredResult) -> bool {
    matches!(
        result,
        ChatStructuredResult::ScoreQuery { records, .. }
            if !records.is_empty() && records.iter().all(|record| record.subject_type == "未区分")
    )
}

fn enrich_memory_from_history(memory: &mut ResolvedMemory, history: &[ConversationMessage]) {
    for message in history.iter().rev() {
        merge_memory_from_history_message(memory, message);
        let Some(structured) = &message.structured_payload else {
            continue;
        };
        merge_memory_from_structured(memory, structured);
        if has_minimum_context(memory) {
            break;
        }
    }
}

fn merge_memory_from_history_message(memory: &mut ResolvedMemory, message: &ConversationMessage) {
    if memory.major_name.is_some() || message.role != "user" {
        return;
    }
    if is_admission_policy_query(&message.content)
        || !should_update_major_from_knowledge_query(&message.content)
        || !message_explicitly_names_major_for_memory(&message.content)
    {
        return;
    }
    if let Some(major) = extract_major_phrase(&message.content) {
        memory.major_name = Some(major.clone());
        memory.major_slug = Some(major);
    }
}

fn merge_memory_from_structured(memory: &mut ResolvedMemory, structured: &ChatStructuredResult) {
    match structured {
        ChatStructuredResult::FollowUp {
            pending_intent,
            collected_profile,
            ..
        } => {
            merge_memory(memory, collected_profile);
            memory
                .pending_intent
                .get_or_insert_with(|| pending_intent.clone());
        }
        ChatStructuredResult::ScoreQuery {
            major_name,
            province,
            subject_type,
            ..
        } => {
            memory.major_name.get_or_insert_with(|| major_name.clone());
            memory.major_slug.get_or_insert_with(|| major_name.clone());
            memory.province_name.get_or_insert_with(|| province.clone());
            if let Some(subject_type) = subject_type {
                memory
                    .subject_type
                    .get_or_insert_with(|| subject_type.clone());
            }
            memory.pending_intent.get_or_insert(ChatIntent::ScoreQuery);
        }
        ChatStructuredResult::ProbabilityAssessment { assessment } => {
            if memory.province_name.is_none() {
                memory.province_name = assessment
                    .get("province")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned);
            }
            if memory.subject_type.is_none() {
                memory.subject_type = assessment
                    .get("subjectType")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned);
            }
            if memory.major_name.is_none() {
                memory.major_name = assessment
                    .get("major")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned);
                memory.major_slug = memory.major_name.clone();
            }
            if memory.score.is_none() {
                memory.score = assessment.get("score").and_then(|value| value.as_f64());
            }
            if memory.rank.is_none() {
                memory.rank = assessment.get("rank").and_then(|value| value.as_f64());
            }
            memory
                .pending_intent
                .get_or_insert(ChatIntent::ProbabilityAssessment);
        }
        ChatStructuredResult::KnowledgeAnswer {
            query,
            vector_chunks,
            ..
        } => {
            memory
                .pending_intent
                .get_or_insert(ChatIntent::KnowledgeAnswer);
            if !is_admission_policy_query(query) && should_update_major_from_knowledge_query(query)
            {
                let resolved_major = major_name_from_vector_chunks(vector_chunks).or_else(|| {
                    message_explicitly_names_major_for_memory(query)
                        .then(|| extract_major_phrase(query))
                        .flatten()
                });
                if let Some(resolved_major) = resolved_major {
                    let should_replace = memory
                        .major_name
                        .as_deref()
                        .is_none_or(|current| major_alias_matches(current, &resolved_major));
                    if should_replace {
                        memory.major_name = Some(resolved_major.clone());
                        memory.major_slug = Some(resolved_major);
                    }
                }
            }
        }
        ChatStructuredResult::ProvinceMajorList {
            province,
            subject_type,
            ..
        } => {
            memory.province_name.get_or_insert_with(|| province.clone());
            if let Some(subject_type) = subject_type {
                memory
                    .subject_type
                    .get_or_insert_with(|| subject_type.clone());
            }
            memory
                .pending_intent
                .get_or_insert(ChatIntent::KnowledgeAnswer);
        }
        ChatStructuredResult::MajorProvinceList {
            major_name,
            subject_type,
            ..
        } => {
            memory.major_name.get_or_insert_with(|| major_name.clone());
            memory.major_slug.get_or_insert_with(|| major_name.clone());
            if let Some(subject_type) = subject_type {
                memory
                    .subject_type
                    .get_or_insert_with(|| subject_type.clone());
            }
            memory
                .pending_intent
                .get_or_insert(ChatIntent::KnowledgeAnswer);
        }
        ChatStructuredResult::MajorDisambiguation {
            pending_intent,
            candidates,
            ..
        } => {
            memory
                .pending_intent
                .get_or_insert_with(|| pending_intent.clone());
            if candidates.len() == 1 {
                if let Some(candidate) = candidates.first() {
                    memory.major_name = Some(candidate.name.clone());
                    memory.major_slug = Some(candidate.slug.clone());
                }
            }
        }
        ChatStructuredResult::EvidenceBundle { results, .. } => {
            let has_probability = results
                .iter()
                .any(|result| matches!(result, ChatStructuredResult::ProbabilityAssessment { .. }));
            for result in results {
                merge_memory_from_structured(memory, result);
            }
            if has_probability && memory.pending_intent.is_none() {
                memory.pending_intent = Some(ChatIntent::ProbabilityAssessment);
            }
        }
        ChatStructuredResult::GeneralAnswer {
            collected_profile, ..
        } => {
            merge_memory(memory, collected_profile);
        }
        ChatStructuredResult::Greeting { .. } | ChatStructuredResult::FallbackReply { .. } => {}
    }
}

fn merge_memory(target: &mut ResolvedMemory, source: &ResolvedMemory) {
    if target.province_code.is_none() {
        target.province_code = source.province_code.clone();
    }
    if target.province_name.is_none() {
        target.province_name = source.province_name.clone();
    }
    if target.subject_type.is_none() {
        target.subject_type = source.subject_type.clone();
    }
    if target.score.is_none() {
        target.score = source.score;
    }
    if target.rank.is_none() {
        target.rank = source.rank;
    }
    if target.major_slug.is_none() {
        target.major_slug = source.major_slug.clone();
    }
    if target.major_name.is_none() {
        target.major_name = source.major_name.clone();
    }
    if target.intended_majors.is_empty() {
        target.intended_majors = source.intended_majors.clone();
    }
    if target.pending_intent.is_none() {
        target.pending_intent = source.pending_intent.clone();
    }
}

fn has_minimum_context(memory: &ResolvedMemory) -> bool {
    memory.province_name.is_some()
        && memory.subject_type.is_some()
        && memory.major_name.is_some()
        && memory.score.is_some()
}

fn apply_contextual_route(
    route: retrieval::RouteDecision,
    message: &str,
    memory: &ResolvedMemory,
) -> retrieval::RouteDecision {
    if asks_major_admission_province_list(message) {
        return retrieval::RouteDecision {
            intent: RetrievalIntent::KnowledgeAnswer,
            must_use_tools: true,
            reason: "专业招生省份列表需要查询录取统计覆盖关系。".to_owned(),
        };
    }

    if asks_province_admission_major_list(message) {
        return retrieval::RouteDecision {
            intent: RetrievalIntent::KnowledgeAnswer,
            must_use_tools: true,
            reason: "省份招生专业列表需要查询分省招生计划或录取统计兜底。".to_owned(),
        };
    }

    if !matches!(route.intent, RetrievalIntent::GeneralAnswer) {
        return route;
    }

    if extract_score(message).is_some() && has_score_context(memory) {
        return retrieval::RouteDecision {
            intent: RetrievalIntent::ProbabilityAssessment,
            must_use_tools: true,
            reason: "短句包含分数，并可从上下文继承省份、科类和专业。".to_owned(),
        };
    }

    if extract_score(message).is_some()
        && explicit_major_text(message).is_some()
        && (memory.province_name.is_some() || memory.province_code.is_some())
        && matches!(
            memory.pending_intent,
            Some(ChatIntent::ProbabilityAssessment)
        )
    {
        return retrieval::RouteDecision {
            intent: RetrievalIntent::ProbabilityAssessment,
            must_use_tools: true,
            reason: "短句包含新专业和分数，并继承上一轮概率评估意图。".to_owned(),
        };
    }

    if extract_known_province(message).is_some()
        && memory.major_name.is_some()
        && matches!(
            memory.pending_intent,
            Some(ChatIntent::ProbabilityAssessment)
        )
        && memory.score.is_some()
        && memory.subject_type.is_some()
    {
        return retrieval::RouteDecision {
            intent: RetrievalIntent::ProbabilityAssessment,
            must_use_tools: true,
            reason: "短句显式更换省份，并继承上一轮概率评估画像。".to_owned(),
        };
    }

    if extract_known_province(message).is_some() && memory.major_name.is_some() {
        return retrieval::RouteDecision {
            intent: RetrievalIntent::ScoreQuery,
            must_use_tools: true,
            reason: "短句包含省份，并可从上下文继承专业。".to_owned(),
        };
    }

    if is_short_follow_up(message) {
        if let Some(intent) = &memory.pending_intent {
            let mapped = match intent {
                ChatIntent::ScoreQuery => Some(RetrievalIntent::ScoreQuery),
                ChatIntent::ProbabilityAssessment => Some(RetrievalIntent::ProbabilityAssessment),
                ChatIntent::KnowledgeAnswer => Some(RetrievalIntent::KnowledgeAnswer),
                _ => None,
            };
            if let Some(intent) = mapped {
                return retrieval::RouteDecision {
                    intent,
                    must_use_tools: true,
                    reason: "短句续问继承上一轮招生咨询意图。".to_owned(),
                };
            }
        }
    }

    if matches!(memory.pending_intent, Some(ChatIntent::KnowledgeAnswer))
        && looks_like_knowledge_follow_up(message)
    {
        return retrieval::RouteDecision {
            intent: RetrievalIntent::KnowledgeAnswer,
            must_use_tools: true,
            reason: "知识类连续追问需要结合上一轮主题继续检索。".to_owned(),
        };
    }

    route
}

fn has_score_context(memory: &ResolvedMemory) -> bool {
    (memory.province_name.is_some() || memory.province_code.is_some())
        && memory.subject_type.is_some()
        && (memory.major_name.is_some() || memory.major_slug.is_some())
}

fn is_short_follow_up(message: &str) -> bool {
    let trimmed = message.trim();
    trimmed.chars().count() <= 14
        || trimmed.ends_with("呢？")
        || trimmed.ends_with("呢")
        || trimmed.contains("继续")
        || trimmed.contains("解读")
}

fn is_major_switch_message(message: &str) -> bool {
    let trimmed = message.trim();
    (trimmed.starts_with("那") || trimmed.starts_with("换成") || trimmed.starts_with("改成"))
        && trimmed.chars().count() <= 18
        && !contains_policy_program_term(trimmed)
        && !asks_province_admission_major_list(trimmed)
}

fn extract_switch_major_query(message: &str) -> Option<String> {
    let mut text = message
        .trim()
        .trim_matches(['，', ',', '。', '？', '?', '！', '!', ' '])
        .to_owned();
    for prefix in ["换成", "改成", "那", "再看", "看看"] {
        if let Some(stripped) = text.strip_prefix(prefix) {
            text = stripped.to_owned();
            break;
        }
    }
    text = text
        .trim()
        .trim_end_matches('呢')
        .trim_end_matches("专业")
        .trim_matches(['，', ',', '。', '？', '?', '！', '!', ' '])
        .to_owned();
    if text.chars().count() >= 2
        && text.chars().count() <= 16
        && extract_score(&text).is_none()
        && !text.contains('分')
        && !text.contains("能上")
        && !text.contains("能不能")
        && !text.contains("能报")
        && !text.contains("稳吗")
        && !text.contains("稳不稳")
        && !text.contains("招生")
        && !text.contains("概率")
        && !text.contains("分数")
        && extract_known_province(&text).is_none()
    {
        Some(text)
    } else {
        None
    }
}

fn asks_major_group_without_college(message: &str) -> bool {
    let asks_group = ["有哪些专业", "有什么专业", "有啥专业", "开设哪些专业"]
        .iter()
        .any(|item| message.contains(item));
    asks_group
        && !contains_policy_program_term(message)
        && !message.contains("学院")
        && !message.contains("招生")
        && extract_known_province(message).is_none()
}

fn contains_policy_program_term(message: &str) -> bool {
    [
        "公费师范",
        "公费师范生",
        "专项计划",
        "少数民族预科",
        "地方专项",
        "国家专项",
        "优师",
    ]
    .iter()
    .any(|item| message.contains(item))
}

fn asks_province_admission_major_list(message: &str) -> bool {
    extract_known_province(message).is_some()
        && (message.contains("招生") || message.contains("招哪些") || message.contains("招什么"))
        && (message.contains("专业") || message.contains("哪些") || message.contains("什么"))
}

fn asks_major_admission_province_list(message: &str) -> bool {
    extract_known_province(message).is_none()
        && contains_any_text(
            message,
            &["哪些省", "哪些省份", "哪个省", "哪些地区", "省份"],
        )
        && contains_any_text(message, &["招生", "招收", "招", "录取记录"])
        && !asks_major_group_without_college(message)
}

fn looks_like_knowledge_follow_up(message: &str) -> bool {
    [
        "有没有",
        "有吗",
        "课程",
        "实践",
        "学分",
        "毕业",
        "培养",
        "目标",
        "要求",
        "环节",
        "换成",
        "这些课",
        "怎么安排",
        "占多少",
        "再说说",
    ]
    .iter()
    .any(|item| message.contains(item))
        || asks_major_fit_context(message)
}

fn contextual_knowledge_query(message: &str, memory: &ResolvedMemory) -> String {
    if is_broad_school_or_campus_query(message) || is_school_level_fact_query(message) {
        return message.to_owned();
    }
    let Some(major) = memory.major_name.as_deref() else {
        return enrich_major_fit_query(message.to_owned());
    };
    if message.contains(major) || major_alias_matches(message, major) {
        return enrich_major_fit_query(message.to_owned());
    }
    if extract_major_phrase(message)
        .as_deref()
        .is_some_and(|current_major| {
            !major_alias_matches(current_major, major)
                && message_explicitly_names_major_for_memory(message)
                && !comparison_uses_contextual_major(message)
        })
    {
        return enrich_major_fit_query(message.to_owned());
    }
    if looks_like_knowledge_follow_up(message) || is_short_follow_up(message) {
        enrich_major_fit_query(format!("{major} {message}"))
    } else {
        enrich_major_fit_query(message.to_owned())
    }
}

fn enrich_major_fit_query(query: String) -> String {
    if asks_major_fit_context(&query) && !contains_any_text(&query, &["培养目标", "毕业要求"])
    {
        format!("{query} 培养目标 毕业要求 实践环节")
    } else {
        query
    }
}

fn contextual_knowledge_query_with_history(
    message: &str,
    memory: &ResolvedMemory,
    history: &[ConversationMessage],
) -> String {
    let query = contextual_knowledge_query(message, memory);
    if is_broad_school_or_campus_query(message) || is_school_level_fact_query(message) {
        return query;
    }
    if query != message || !(looks_like_knowledge_follow_up(message) || is_short_follow_up(message))
    {
        return query;
    }
    if extract_major_phrase(message).is_some() && !comparison_uses_contextual_major(message) {
        return query;
    }

    history
        .iter()
        .rev()
        .filter(|item| item.role == "user")
        .filter(|item| !is_admission_policy_query(&item.content))
        .filter(|item| should_update_major_from_knowledge_query(&item.content))
        .find_map(|item| extract_major_phrase(&item.content))
        .map(|major| format!("{major}专业 {message}"))
        .unwrap_or(query)
}

fn should_force_training_plan_major_focus(query: &str, memory: &ResolvedMemory) -> bool {
    memory
        .major_name
        .as_deref()
        .is_some_and(|major| !major.trim().is_empty())
        && !is_admission_policy_query(query)
        && contains_any_text(
            query,
            &[
                "培养方案",
                "培养目标",
                "毕业要求",
                "毕业条件",
                "毕业需要",
                "主要课程",
                "课程",
                "实践环节",
                "教育实习",
                "专业实践",
                "第二课堂",
                "创新实践",
                "毕业创作",
                "毕业论文",
                "学分",
                "有没有",
                "怎么安排",
                "适合",
                "匹配",
                "契合",
            ],
        )
}

fn render_major_disambiguation_reply(result: &ChatStructuredResult) -> String {
    let ChatStructuredResult::MajorDisambiguation { candidates, .. } = result else {
        return "我需要先确认你想了解的具体专业。".to_owned();
    };
    if candidates.is_empty() {
        return "我需要先确认你想了解的具体专业。你可以说专业全称，或补充学院、方向关键词。"
            .to_owned();
    }
    let names = candidates
        .iter()
        .map(|candidate| candidate.name.as_str())
        .collect::<Vec<_>>();
    format!(
        "我先帮你把可能相关的专业列出来：{}。你可以指定其中一个专业，我再继续查分数线、招生情况或培养方案。",
        names.join("、")
    )
}

fn render_probability_answer(result: &ChatStructuredResult) -> String {
    let ChatStructuredResult::ProbabilityAssessment { assessment } = result else {
        return "我已经查询录取统计，可以结合你的分数做参考判断。".to_owned();
    };
    let province = assessment
        .get("province")
        .and_then(|value| value.as_str())
        .unwrap_or("对应省份");
    let subject_type = assessment
        .get("subjectType")
        .and_then(|value| value.as_str())
        .unwrap_or("对应科类");
    let requested_subject_type = assessment
        .get("requestedSubjectType")
        .and_then(|value| value.as_str());
    let subject_text = if subject_type == "未区分" {
        if let Some(requested) = requested_subject_type {
            format!("未区分科类/普通类记录（你提到的{requested}未在统计表中单列）")
        } else {
            "未区分科类/普通类记录".to_owned()
        }
    } else {
        subject_type.to_owned()
    };
    let score = assessment
        .get("score")
        .and_then(|value| value.as_f64())
        .map(|value| format!("{value:.0}分"))
        .unwrap_or_else(|| "你的分数".to_owned());
    let major = assessment
        .get("major")
        .and_then(|value| value.as_str())
        .unwrap_or("目标专业");
    let probability = assessment
        .get("probability")
        .and_then(|value| value.as_i64());
    let level = assessment
        .get("level")
        .and_then(|value| value.as_str())
        .unwrap_or("reference");
    let confidence = assessment
        .get("confidence")
        .and_then(|value| value.as_str())
        .unwrap_or("low");
    let summary_text = assessment
        .get("summary")
        .and_then(|value| value.as_str())
        .unwrap_or("该结果为历史数据推断，仅供参考。");
    let factor_text = assessment
        .get("factors")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .take(3)
                .collect::<Vec<_>>()
                .join("；")
        })
        .filter(|value| !value.is_empty());
    let score_summary = assessment.get("scoreSummary");
    let record_count = score_summary
        .and_then(|value| value.get("recordCount"))
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    if record_count == 0 {
        let probability_text = probability
            .map(|value| format!("系统给出的粗略参考概率为 {value}%（置信度 {confidence}）"))
            .unwrap_or_else(|| "当前只能做低置信度参考".to_owned());
        return format!(
            "我已按{province}、{subject_text}、{score}和{major}查询历年录取统计，但暂时没有找到可直接对比的分专业记录。{probability_text}，不能当成目标专业的准确录取概率。建议继续核对专业全称、科类和当年招生计划。"
        );
    }
    let latest_year = score_summary
        .and_then(|value| value.get("latestYear"))
        .and_then(|value| value.as_i64());
    let latest_min = score_summary
        .and_then(|value| value.get("latestMinScore"))
        .and_then(|value| value.as_i64());
    match (latest_year, latest_min) {
        (Some(year), Some(min_score)) => {
            let probability_text = probability
                .map(|value| {
                    format!(
                        "确定性概率引擎给出的参考概率为 {value}%（{level}，置信度 {confidence}）"
                    )
                })
                .unwrap_or_else(|| "已完成基于历年分数线的参考评估".to_owned());
            format!(
                "我按{province}、{subject_text}、{major}查询到了历年录取统计。最近可用记录是 {year} 年最低分 {min_score} 分，你的分数是{score}。{probability_text}。{summary_text}{}这只能作为历史参考，最终还要结合当年招生计划、报考热度和省级投档规则判断。",
                factor_text
                    .map(|value| format!("主要依据：{value}。"))
                    .unwrap_or_default()
            )
        }
        _ => {
            let probability_text = probability
                .map(|value| format!("参考概率为 {value}%（{level}，置信度 {confidence}）"))
                .unwrap_or_else(|| "可以作为历史分数线对比参考".to_owned());
            format!(
                "我按{province}、{subject_text}、{major}查询到了 {record_count} 条历年录取记录。你的分数是{score}，{probability_text}。最终还要结合当年招生计划和报考热度判断。"
            )
        }
    }
}

fn render_evidence_bundle_answer(result: &ChatStructuredResult) -> String {
    let ChatStructuredResult::EvidenceBundle { results, .. } = result else {
        return match result {
            ChatStructuredResult::ScoreQuery { .. } => render_score_answer(result),
            ChatStructuredResult::ProbabilityAssessment { .. } => render_probability_answer(result),
            ChatStructuredResult::KnowledgeAnswer { .. } => render_knowledge_answer(result),
            ChatStructuredResult::ProvinceMajorList { .. } => {
                render_province_major_list_answer(result)
            }
            ChatStructuredResult::MajorProvinceList { .. } => {
                render_major_province_list_answer(result)
            }
            _ => "我已经合并查询了相关招生证据。".to_owned(),
        };
    };

    let mut parts = Vec::new();
    for item in results {
        match item {
            ChatStructuredResult::ProbabilityAssessment { .. } => {
                parts.push(render_probability_answer(item));
            }
            ChatStructuredResult::ScoreQuery { .. } => {
                parts.push(render_score_answer(item));
            }
            ChatStructuredResult::KnowledgeAnswer { .. } => {
                parts.push(render_knowledge_answer(item));
            }
            ChatStructuredResult::ProvinceMajorList { .. } => {
                parts.push(render_province_major_list_answer(item));
            }
            ChatStructuredResult::MajorProvinceList { .. } => {
                parts.push(render_major_province_list_answer(item));
            }
            ChatStructuredResult::MajorDisambiguation { .. } => {
                parts.push(render_major_disambiguation_reply(item));
            }
            _ => {}
        }
    }

    if parts.is_empty() {
        "我已经合并查询了相关招生证据。".to_owned()
    } else {
        parts.join("\n\n")
    }
}

fn finalize_reply(
    reply: String,
    structured_result: &ChatStructuredResult,
    memory: &ResolvedMemory,
) -> String {
    let reply = ensure_reply_mentions_confirmed_major(reply, structured_result, memory);
    let reply = ensure_reply_mentions_probability_score(reply, structured_result);
    let reply = sanitize_unspecified_subject_false_negative(reply, structured_result);
    let reply = sanitize_unspecified_subject_score_boundary(reply, structured_result);
    sanitize_high_risk_facts(reply, structured_result)
}

fn sanitize_unspecified_subject_false_negative(
    reply: String,
    structured_result: &ChatStructuredResult,
) -> String {
    if probability_subject_type_is_unspecified(structured_result) && !reply.contains("未区分") {
        return render_evidence_bundle_answer(structured_result);
    }
    let Some(requested_subject) = probability_uses_unspecified_subject_records(structured_result)
    else {
        return reply;
    };
    if !contains_any_text(&reply, &[requested_subject.as_str()])
        || !contains_any_text(
            &reply,
            &[
                "暂未找到",
                "暂未收录",
                "没有找到",
                "未找到",
                "缺少",
                "暂无",
                "没有直接",
            ],
        )
    {
        return reply;
    }
    render_evidence_bundle_answer(structured_result)
}

fn probability_subject_type_is_unspecified(structured_result: &ChatStructuredResult) -> bool {
    match structured_result {
        ChatStructuredResult::ProbabilityAssessment { assessment } => {
            let record_count = assessment
                .get("scoreSummary")
                .and_then(|value| value.get("recordCount"))
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            record_count > 0
                && assessment
                    .get("subjectType")
                    .and_then(|value| value.as_str())
                    .is_some_and(|value| value == "未区分")
        }
        ChatStructuredResult::EvidenceBundle { results, .. } => {
            results.iter().any(probability_subject_type_is_unspecified)
        }
        _ => false,
    }
}

fn probability_uses_unspecified_subject_records(
    structured_result: &ChatStructuredResult,
) -> Option<String> {
    match structured_result {
        ChatStructuredResult::ProbabilityAssessment { assessment } => {
            let subject_type = assessment
                .get("subjectType")
                .and_then(|value| value.as_str());
            let requested_subject = assessment
                .get("requestedSubjectType")
                .and_then(|value| value.as_str());
            let record_count = assessment
                .get("scoreSummary")
                .and_then(|value| value.get("recordCount"))
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            match (subject_type, requested_subject, record_count) {
                (Some("未区分"), Some(requested), count) if count > 0 => {
                    Some(requested.to_owned())
                }
                _ => None,
            }
        }
        ChatStructuredResult::EvidenceBundle { results, .. } => results
            .iter()
            .find_map(probability_uses_unspecified_subject_records),
        _ => None,
    }
}

fn sanitize_unspecified_subject_score_boundary(
    reply: String,
    structured_result: &ChatStructuredResult,
) -> String {
    let Some(requested_subject) = score_query_uses_unspecified_subject_records(structured_result)
    else {
        return reply;
    };
    if !reply.contains("未区分") && reply.contains(&requested_subject) {
        return render_evidence_bundle_answer(structured_result);
    }
    reply
}

fn score_query_uses_unspecified_subject_records(
    structured_result: &ChatStructuredResult,
) -> Option<String> {
    match structured_result {
        ChatStructuredResult::ScoreQuery {
            subject_type,
            records,
            diagnostics,
            ..
        } => {
            if subject_type.as_deref() != Some("未区分") || records.is_empty() {
                return None;
            }
            diagnostics
                .as_ref()
                .and_then(|value| value.get("requestedSubjectType"))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        }
        ChatStructuredResult::EvidenceBundle { results, .. } => results
            .iter()
            .find_map(score_query_uses_unspecified_subject_records),
        _ => None,
    }
}

fn sanitize_high_risk_facts(reply: String, structured_result: &ChatStructuredResult) -> String {
    let reply = sanitize_unverified_art_formula(reply, structured_result);
    let reply = sanitize_unverified_admissions_phones(reply, structured_result);
    sanitize_evidence_backed_term_typos(reply, structured_result)
}

fn sanitize_evidence_backed_term_typos(
    reply: String,
    structured_result: &ChatStructuredResult,
) -> String {
    let evidence_text = structured_result_text(structured_result);
    if !(reply.contains("行知") || evidence_text.contains("行知")) {
        return reply;
    }

    reply.replace("言行班", "行知班")
        .replace("知行班", "行知班")
}

fn sanitize_unverified_art_formula(
    reply: String,
    structured_result: &ChatStructuredResult,
) -> String {
    if !contains_art_4060_formula(&reply) || evidence_contains_art_4060_formula(structured_result) {
        return reply;
    }

    let mut sanitized = String::new();
    for sentence in split_reply_sentences(&reply) {
        if contains_art_4060_formula(sentence) {
            continue;
        }
        sanitized.push_str(sentence);
    }
    if sanitized.trim().is_empty() {
        "艺术类录取规则需要以学校招生简章和生源省级招生主管部门公布的投档规则为准；当前证据不足以支持具体折算公式。".to_owned()
    } else {
        sanitized
    }
}

fn sanitize_unverified_admissions_phones(
    reply: String,
    structured_result: &ChatStructuredResult,
) -> String {
    let phones = extract_harbin_phone_numbers(&reply);
    if phones.is_empty() {
        return reply;
    }
    let mut allowed = extract_harbin_phone_numbers(&structured_result_text(structured_result));
    allowed.push(official_admissions_phone().to_owned());
    allowed.sort();
    allowed.dedup();

    let mut sanitized = reply;
    for phone in phones {
        if allowed
            .iter()
            .any(|allowed_phone| phone_digits(allowed_phone) == phone_digits(&phone))
        {
            continue;
        }
        sanitized = sanitized.replace(&phone, official_admissions_phone());
    }
    sanitized
}

fn official_admissions_phone() -> &'static str {
    "0451-88067377"
}

fn structured_result_text(structured_result: &ChatStructuredResult) -> String {
    serde_json::to_string(structured_result).unwrap_or_default()
}

fn contains_art_4060_formula(text: &str) -> bool {
    let normalized = text
        .replace(' ', "")
        .replace('＋', "+")
        .replace('×', "*")
        .replace('％', "%");
    let has_art_score_source = normalized.contains("专业课")
        || normalized.contains("专业成绩")
        || normalized.contains("专业统考")
        || normalized.contains("统考成绩")
        || normalized.contains("术科")
        || normalized.contains("美术类")
        || normalized.contains("音乐类")
        || normalized.contains("舞蹈类")
        || normalized.contains("艺术类");
    (normalized.contains("40%") || normalized.contains("0.4"))
        && (normalized.contains("60%") || normalized.contains("0.6"))
        && (normalized.contains("文化课") || normalized.contains("文化成绩"))
        && has_art_score_source
}

fn evidence_contains_art_4060_formula(structured_result: &ChatStructuredResult) -> bool {
    contains_art_4060_formula(&structured_result_text(structured_result))
}

fn split_reply_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;
    for (index, ch) in text.char_indices() {
        if matches!(ch, '。' | '！' | '？' | '\n') {
            let end = index + ch.len_utf8();
            sentences.push(&text[start..end]);
            start = end;
        }
    }
    if start < text.len() {
        sentences.push(&text[start..]);
    }
    sentences
}

fn extract_harbin_phone_numbers(text: &str) -> Vec<String> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut phones = Vec::new();
    let mut index = 0;
    while index + 4 <= chars.len() {
        if chars[index] == '0'
            && chars.get(index + 1) == Some(&'4')
            && chars.get(index + 2) == Some(&'5')
            && chars.get(index + 3) == Some(&'1')
        {
            let mut end = index + 4;
            while end < chars.len()
                && (chars[end].is_ascii_digit()
                    || matches!(chars[end], '-' | '–' | '—' | ' ' | '\u{00a0}'))
            {
                end += 1;
            }
            let candidate = chars[index..end].iter().collect::<String>();
            if phone_digits(&candidate).len() == 12 {
                phones.push(candidate);
            }
            index = end;
        } else {
            index += 1;
        }
    }
    phones.sort();
    phones.dedup();
    phones
}

fn phone_digits(phone: &str) -> String {
    phone.chars().filter(|ch| ch.is_ascii_digit()).collect()
}

fn ensure_reply_mentions_confirmed_major(
    reply: String,
    structured_result: &ChatStructuredResult,
    _memory: &ResolvedMemory,
) -> String {
    let major = target_major_from_structured(structured_result)
        .filter(|value| is_plausible_major_text(value));
    let Some(major) = major else {
        return reply;
    };
    if reply.contains(&major) {
        return reply;
    }
    let normalized_reply = normalize_major_alias(&reply);
    let normalized_major = normalize_major_alias(&major);
    let normalized_root =
        normalize_major_alias(major.split(['（', '(']).next().unwrap_or(major.as_str()));
    if (!normalized_major.is_empty() && normalized_reply.contains(&normalized_major))
        || (!normalized_root.is_empty() && normalized_reply.contains(&normalized_root))
    {
        return reply;
    }
    format!("关于{major}，{reply}")
}

fn ensure_reply_mentions_probability_score(
    reply: String,
    structured_result: &ChatStructuredResult,
) -> String {
    let Some(score) = probability_score_from_structured(structured_result) else {
        return reply;
    };
    let score_text = format_score_for_reply(score);
    if reply.contains(&score_text) || reply.contains(&format!("{score_text}分")) {
        return reply;
    }
    format!("本轮按考生{score_text}分进行参考分析。{reply}")
}

fn probability_score_from_structured(structured_result: &ChatStructuredResult) -> Option<f64> {
    match structured_result {
        ChatStructuredResult::ProbabilityAssessment { assessment } => {
            assessment.get("score").and_then(|value| value.as_f64())
        }
        ChatStructuredResult::EvidenceBundle { results, .. } => {
            results.iter().find_map(probability_score_from_structured)
        }
        _ => None,
    }
}

fn format_score_for_reply(score: f64) -> String {
    if (score.fract()).abs() < f64::EPSILON {
        format!("{score:.0}")
    } else {
        format!("{score}")
    }
}

fn target_major_from_structured(result: &ChatStructuredResult) -> Option<String> {
    match result {
        ChatStructuredResult::ScoreQuery { major_name, .. } => Some(major_name.clone()),
        ChatStructuredResult::MajorProvinceList { major_name, .. } => Some(major_name.clone()),
        ChatStructuredResult::ProbabilityAssessment { assessment } => assessment
            .get("major")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
        ChatStructuredResult::KnowledgeAnswer {
            query,
            vector_chunks,
            ..
        } if should_update_major_from_knowledge_query(query) => {
            major_name_from_vector_chunks(vector_chunks).or_else(|| extract_major_phrase(query))
        }
        ChatStructuredResult::EvidenceBundle { results, .. } => {
            results.iter().find_map(target_major_from_structured)
        }
        ChatStructuredResult::MajorDisambiguation { candidates, .. } if candidates.len() == 1 => {
            candidates.first().map(|candidate| candidate.name.clone())
        }
        _ => None,
    }
}

fn major_name_from_vector_chunks(chunks: &[domain::VectorChunkEvidence]) -> Option<String> {
    chunks
        .iter()
        .filter_map(|chunk| {
            chunk
                .metadata
                .get("majorName")
                .and_then(|value| value.as_str())
        })
        .find(|value| is_plausible_major_text(value))
        .map(ToOwned::to_owned)
}

fn should_synthesize(structured_result: &ChatStructuredResult) -> bool {
    matches!(
        structured_result,
        ChatStructuredResult::ScoreQuery { .. }
            | ChatStructuredResult::ProbabilityAssessment { .. }
            | ChatStructuredResult::KnowledgeAnswer { .. }
            | ChatStructuredResult::MajorDisambiguation { .. }
            | ChatStructuredResult::EvidenceBundle { .. }
            | ChatStructuredResult::GeneralAnswer { .. }
    )
}

fn render_greeting_answer(message: &str) -> String {
    if contains_any_text(message, &["擅长", "能做什么", "可以做什么", "会什么"]) {
        return "我比较擅长帮你把哈师大的报考信息讲清楚 😊\n\n比如历年录取分数和位次、招生政策、专业培养方案、核心课程、毕业要求、校区住宿、校园生活这些都可以问我。涉及分数线、计划数、录取规则这类关键信息，我会尽量依据学校官方资料来回答，不会随便猜。".to_owned();
    }
    if contains_any_text(
        message,
        &["你是谁", "你是啥", "你是什么", "介绍一下你", "自我介绍"],
    ) {
        return "你好呀～我是哈尔滨师范大学招生智能顾问，也可以把我当成一个报考小助手 😊\n\n你可以问我招生政策、各省录取分数、专业学什么、培养方案怎么安排，也可以聊住宿、食堂、校区和校园生活。你直接说想了解的问题就行，我会尽量用清楚、靠谱、好懂的方式回答你。".to_owned();
    }
    "你好呀～我是哈师大招生智能顾问 😊\n\n想查录取分数、招生政策、专业课程、培养方案，或者想了解校园生活，都可以直接问我。".to_owned()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    text.chars().take(max_chars).collect::<String>()
}

fn enrich_memory_from_message(memory: &mut ResolvedMemory, message: &str) {
    if let Some(score) = extract_score(message) {
        memory.score = Some(score);
    }
    if let Some(subject_type) = extract_subject_type(message) {
        memory.subject_type = Some(subject_type);
    }
    if let Some(province) = extract_known_province(message) {
        memory.province_name = Some(province);
    }
    if should_extract_major_phrase(message)
        && message_explicitly_names_major_for_memory(message)
        && !(asks_score_comparison(message) && comparison_uses_contextual_major(message))
    {
        if let Some(major) = extract_major_phrase(message) {
            memory.major_name = Some(major.clone());
            memory.major_slug = Some(major);
        }
    }
}

fn should_extract_major_phrase(message: &str) -> bool {
    [
        "录取线",
        "分数线",
        "近三年",
        "近五年",
        "多少分",
        "几分",
        "能上",
        "能进",
        "能报",
        "可以上",
        "可以进",
        "可以报",
        "录取情况",
        "录取数据",
        "录取统计",
        "培养方案",
        "培养目标",
        "毕业条件",
        "毕业要求",
        "毕业需要",
        "课程",
        "学分",
        "实践环节",
        "教育实习",
        "第二课堂",
        "毕业创作",
        "适合",
        "匹配",
        "契合",
        "想当老师",
        "喜欢做实验",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

fn should_update_major_from_knowledge_query(query: &str) -> bool {
    if is_admission_policy_query(query) {
        return false;
    }

    if is_broad_school_or_campus_query(query) || is_school_level_fact_query(query) {
        return false;
    }

    if contains_any_text(
        query,
        &[
            "校园",
            "大学生活",
            "学生生活",
            "食堂",
            "宿舍",
            "住宿",
            "社团",
            "学校介绍",
            "学校简介",
            "学校情况",
            "院校介绍",
        ],
    ) && extract_major_phrase(query).is_none()
    {
        return false;
    }

    should_extract_major_phrase(query)
        || (contains_any_text(
            query,
            &[
                "专业",
                "课程",
                "培养目标",
                "学分",
                "毕业要求",
                "实践环节",
                "培养方案",
                "适合",
                "匹配",
                "契合",
            ],
        ) && extract_major_phrase(query).is_some())
}

fn is_broad_school_or_campus_query(message: &str) -> bool {
    let asks_intro = contains_any_text(message, &["介绍", "简介", "讲讲", "说说", "了解"]);
    let asks_school = contains_any_text(
        message,
        &[
            "学校",
            "院校",
            "哈师大",
            "哈尔滨师范大学",
            "校园",
            "校区",
            "大学",
        ],
    );
    let asks_specific_program = contains_any_text(
        message,
        &[
            "专业",
            "学院",
            "课程",
            "培养",
            "录取",
            "分数",
            "位次",
            "招生计划",
            "招生简章",
        ],
    );
    asks_intro && asks_school && !asks_specific_program
}

fn is_school_level_fact_query(message: &str) -> bool {
    let mentions_school = contains_any_text(
        message,
        &["学校", "院校", "哈师大", "哈尔滨师范大学", "贵校", "你校"],
    );
    if !mentions_school {
        return contains_any_text(message, &["校训", "校风", "学校章程"]);
    }

    contains_any_text(
        message,
        &[
            "校训",
            "校风",
            "校规",
            "学校章程",
            "办学定位",
            "办学特色",
            "学校特色",
            "学术不端",
            "校园网",
            "创意市集",
        ],
    )
}

fn message_explicitly_names_major_for_memory(message: &str) -> bool {
    is_major_switch_message(message)
        || contains_any_text(
            message,
            &[
                "专业",
                "培养方案",
                "培养目标",
                "毕业条件",
                "毕业要求",
                "毕业需要",
                "录取线",
                "分数线",
                "近三年",
                "近五年",
                "2021",
                "2022",
                "2023",
                "2024",
                "2025",
            ],
        )
}

fn is_admission_policy_query(query: &str) -> bool {
    contains_any_text(
        query,
        &[
            "招生简章",
            "招生章程",
            "录取规则",
            "专业志愿",
            "专业级差",
            "级差",
            "服从调剂",
            "调剂",
            "退档",
            "同分",
            "优先录取",
            "分数相同",
            "成绩相同",
            "体检",
            "语种",
            "外语语种",
            "单科成绩",
            "选考科目",
            "选考",
            "选科",
            "招生计划",
            "招生电话",
            "咨询电话",
            "官网",
        ],
    ) || contains_policy_program_term(query)
}

fn extract_score(message: &str) -> Option<f64> {
    let chars = message.chars().collect::<Vec<_>>();
    for index in 0..chars.len() {
        if chars.get(index + 3) == Some(&'分') {
            let candidate = chars[index..index + 3].iter().collect::<String>();
            if let Ok(score) = candidate.parse::<f64>() {
                return Some(score);
            }
        }
    }
    None
}

fn extract_subject_type(message: &str) -> Option<String> {
    for item in ["物理类", "历史类", "理科", "文科", "未区分"] {
        if message.contains(item) {
            return Some(item.to_owned());
        }
    }
    None
}

fn extract_known_province(message: &str) -> Option<String> {
    const PROVINCES: &[&str] = &[
        "北京",
        "天津",
        "河北",
        "山西",
        "内蒙古",
        "辽宁",
        "吉林",
        "黑龙江",
        "上海",
        "江苏",
        "浙江",
        "安徽",
        "福建",
        "江西",
        "山东",
        "河南",
        "湖北",
        "湖南",
        "广东",
        "广西",
        "海南",
        "重庆",
        "四川",
        "贵州",
        "云南",
        "陕西",
        "甘肃",
        "青海",
        "宁夏",
        "新疆",
    ];
    PROVINCES
        .iter()
        .find(|province| message.contains(**province))
        .map(|province| (*province).to_owned())
}

fn extract_year_from_message(message: &str) -> Option<i32> {
    for year in 2021..=2039 {
        if message.contains(&year.to_string()) {
            return Some(year);
        }
    }
    None
}

fn resolve_admission_score_year_for_turn(
    message: &str,
    history: &[ConversationMessage],
    route_intent: &RetrievalIntent,
) -> Option<i32> {
    if let Some(year) = extract_supported_admission_year(message) {
        return Some(year);
    }
    if !matches!(
        route_intent,
        RetrievalIntent::ScoreQuery | RetrievalIntent::ProbabilityAssessment
    ) {
        return None;
    }
    if !(is_short_follow_up(message)
        || extract_known_province(message).is_some()
        || extract_subject_type(message).is_some())
    {
        return None;
    }
    latest_supported_admission_year_from_history(history)
}

fn latest_supported_admission_year_from_history(history: &[ConversationMessage]) -> Option<i32> {
    history
        .iter()
        .rev()
        .filter(|message| message.role == "user")
        .find_map(|message| extract_supported_admission_year(&message.content))
}

fn extract_supported_admission_year(message: &str) -> Option<i32> {
    if contains_supported_admission_year_range(message) {
        return None;
    }
    for year in 2021..=2025 {
        if message.contains(&year.to_string()) {
            return Some(year);
        }
    }
    None
}

fn contains_supported_admission_year_range(message: &str) -> bool {
    [
        "2021到2025",
        "2021年到2025年",
        "2021-2025",
        "2021年-2025年",
        "2021—2025",
        "2021年—2025年",
        "2021至2025",
        "2021年至2025年",
        "21到25",
        "21-25",
        "近五年",
        "五年",
    ]
    .iter()
    .any(|item| message.contains(item))
}

fn extract_major_phrase(message: &str) -> Option<String> {
    if let Some(major) = extract_named_major_before_profession_word(message) {
        return Some(major);
    }

    let marker = [
        "录取线",
        "分数线",
        "近三年",
        "能上",
        "能报",
        "培养方案",
        "培养目标",
        "毕业条件",
        "毕业要求",
        "毕业需要",
        "教育实习",
        "实践环节",
        "第二课堂",
        "毕业创作",
        "课程",
        "学分",
    ]
    .into_iter()
    .filter_map(|marker| message.find(marker).map(|index| (index, marker)))
    .min_by_key(|(index, _)| *index);

    if let Some((index, marker)) = marker {
        let before = message[..index].trim_matches(['，', ',', '。', '？', '?', ' ']);
        let without_profile = before
            .replace("物理类", "")
            .replace("历史类", "")
            .replace("理科", "")
            .replace("文科", "");
        let without_province = strip_known_provinces(&without_profile);
        let cleaned_before = clean_major_candidate_text(&without_province);
        let cleaned = clean_major_candidate_text(
            cleaned_before
                .split(['，', ',', ' '])
                .next_back()
                .unwrap_or("")
                .trim(),
        );
        if is_plausible_major_text(&cleaned) {
            return Some(cleaned);
        }
        let after = message[index + marker.len()..].trim();
        let cleaned_after = clean_major_candidate_text(after);
        if is_plausible_major_text(&cleaned_after) {
            return Some(cleaned_after);
        }
    }
    explicit_major_text(message)
}

fn extract_named_major_before_profession_word(message: &str) -> Option<String> {
    let index = message.find("专业")?;
    let before = message[..index].trim_matches(['，', ',', '。', '？', '?', ' ']);
    if before.ends_with("这个") || before.ends_with("该") || before.is_empty() {
        return None;
    }
    let segment = before
        .split(['，', ',', '。', '？', '?', ' '])
        .next_back()
        .unwrap_or(before);
    let cleaned = clean_major_candidate_text(segment);
    if is_plausible_major_text(&cleaned) {
        Some(cleaned)
    } else {
        None
    }
}

fn explicit_major_text(message: &str) -> Option<String> {
    if looks_like_knowledge_follow_up(message) && !contains_any_text(message, &["换成", "改成"])
    {
        return None;
    }
    if let Some(major) = extract_switch_major_query(message) {
        return Some(major);
    }
    let mut text = message.to_owned();
    if let Some(score) = extract_score(message) {
        text = text.replace(&format!("{score:.0}分"), "");
    }
    for token in [
        "能上",
        "能不能上",
        "能报",
        "概率",
        "录取概率",
        "稳吗",
        "录取线",
        "分数线",
        "最低分",
        "近三年",
        "历年",
        "培养方案",
        "培养目标",
        "毕业条件",
        "毕业要求",
        "毕业需要",
        "讲一下",
        "介绍一下",
        "介绍",
        "一下",
        "是什么",
        "有没有",
        "那",
        "这个专业",
    ] {
        text = text.replace(token, "");
    }
    let cleaned = clean_major_candidate_text(&text);
    if is_plausible_major_text(&cleaned) {
        Some(cleaned)
    } else {
        None
    }
}

fn clean_major_candidate_text(text: &str) -> String {
    let mut cleaned = strip_known_provinces(text)
        .replace("物理类", "")
        .replace("历史类", "")
        .replace("理科", "")
        .replace("文科", "")
        .replace("专业的", "")
        .replace("这个专业", "")
        .replace("该专业", "")
        .replace("主要", "")
        .replace("核心", "");
    for token in [
        "需要",
        "多少",
        "要求",
        "讲",
        "一下",
        "呢",
        "说说",
        "再说说",
        "里的",
        "里",
    ] {
        cleaned = cleaned.replace(token, "");
    }
    for year in 2021..=2039 {
        cleaned = cleaned.replace(&year.to_string(), "");
    }
    cleaned
        .replace("到", "")
        .replace("至", "")
        .trim_end_matches("专业")
        .trim_end_matches('的')
        .trim_matches([
            '，', ',', '。', '？', '?', '！', '!', ' ', '呢', '吗', '啊', '：', ':',
        ])
        .to_owned()
}

fn is_plausible_major_text(text: &str) -> bool {
    text.chars().count() >= 2
        && text.chars().count() <= 24
        && !text.chars().all(|ch| ch.is_ascii_digit())
        && !text.contains('分')
        && !text.contains("能上")
        && !text.contains("能不能")
        && !text.contains("能报")
        && !text.contains("概率")
        && !text.contains("稳吗")
        && !text.contains("稳不稳")
        && !text.contains("招生")
        && !text.contains("是什么")
        && !text.contains("有没有")
        && !matches!(
            text,
            "主要"
                | "有哪些"
                | "有什么"
                | "有啥"
                | "有吗"
                | "怎么样"
                | "怎么安排"
                | "如何安排"
                | "多少"
                | "需要多少"
                | "这个"
                | "该专业"
                | "专业"
                | "一下"
        )
        && !looks_like_knowledge_follow_up(text)
        && !contains_any_text(
            text,
            &[
                "主要课程",
                "课程有哪些",
                "毕业要求",
                "毕业条件",
                "实践环节",
                "学分要求",
                "学分结构",
                "培养目标",
                "怎么安排",
            ],
        )
        && !text.contains("专业有哪些")
        && !text.contains("什么专业")
        && !text.contains("哪些专业")
        && !contains_policy_program_term(text)
}

fn contains_any_text(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn strip_known_provinces(text: &str) -> String {
    const PROVINCES: &[&str] = &[
        "北京",
        "天津",
        "河北",
        "山西",
        "内蒙古",
        "辽宁",
        "吉林",
        "黑龙江",
        "上海",
        "江苏",
        "浙江",
        "安徽",
        "福建",
        "江西",
        "山东",
        "河南",
        "湖北",
        "湖南",
        "广东",
        "广西",
        "海南",
        "重庆",
        "四川",
        "贵州",
        "云南",
        "陕西",
        "甘肃",
        "青海",
        "宁夏",
        "新疆",
    ];
    PROVINCES.iter().fold(text.to_owned(), |current, province| {
        current.replace(province, "")
    })
}

fn major_alias_matches(left: &str, right: &str) -> bool {
    let left = normalize_major_alias(left);
    let right = normalize_major_alias(right);
    !left.is_empty()
        && !right.is_empty()
        && (left == right || left.contains(&right) || right.contains(&left))
}

fn normalize_major_alias(text: &str) -> String {
    strip_known_provinces(text)
        .replace(['（', '）', '(', ')', ' ', '，', ',', '、'], "")
        .replace("物理类", "")
        .replace("历史类", "")
        .replace("理科", "")
        .replace("文科", "")
        .replace("师范类", "")
        .replace("师范", "")
        .replace("专业", "")
}

fn missing_score_fields(memory: &ResolvedMemory) -> Vec<String> {
    let mut fields = Vec::new();
    if memory.province_name.is_none() && memory.province_code.is_none() {
        fields.push("province".to_owned());
    }
    if memory.major_name.is_none() && memory.major_slug.is_none() {
        fields.push("major".to_owned());
    }
    fields
}

fn missing_probability_fields(memory: &ResolvedMemory) -> Vec<String> {
    let mut fields = missing_score_fields(memory);
    if memory.subject_type.is_none() {
        fields.push("subjectType".to_owned());
    }
    if memory.score.is_none() {
        fields.push("score".to_owned());
    }
    fields
}

fn effective_probability_missing_fields(message: &str, memory: &ResolvedMemory) -> Vec<String> {
    let mut fields = missing_probability_fields(memory);
    if fields.len() == 1
        && fields.first().is_some_and(|field| field == "subjectType")
        && (extract_score(message).is_some()
            || memory.score.is_some()
                && matches!(
                    memory.pending_intent,
                    Some(ChatIntent::ProbabilityAssessment)
                ))
        && explicit_major_text(message).is_some()
    {
        fields.clear();
    }
    fields
}

fn render_follow_up(missing: &[String], memory: &ResolvedMemory) -> String {
    let labels = missing
        .iter()
        .map(|field| match field.as_str() {
            "province" => "省份",
            "subjectType" => "科类/选科",
            "score" => "分数",
            "major" => "意向专业",
            _ => field,
        })
        .collect::<Vec<_>>();
    let mut confirmed = Vec::new();
    if let Some(province) = memory
        .province_name
        .as_deref()
        .or(memory.province_code.as_deref())
    {
        confirmed.push(format!("省份是{province}"));
    }
    if let Some(subject_type) = memory.subject_type.as_deref() {
        confirmed.push(format!("科类/选科是{subject_type}"));
    }
    if let Some(score) = memory.score {
        confirmed.push(format!("分数是{score:.0}分"));
    }
    if let Some(major) = memory
        .major_name
        .as_deref()
        .or(memory.major_slug.as_deref())
    {
        confirmed.push(format!("意向专业是{major}"));
    }

    let confirmed_text = if confirmed.is_empty() {
        String::new()
    } else {
        format!("我先记下：{}。", confirmed.join("，"))
    };
    let task_hint = if missing.iter().any(|field| field == "subjectType")
        && memory.province_name.is_some()
        && memory.major_name.is_none()
    {
        "这样我才能按对应科类继续查这个省份当年招生专业。"
    } else {
        "这样我才能继续给你查录取线、概率或专业资料。"
    };
    format!(
        "{confirmed_text}还需要你补充{}，{task_hint}",
        labels.join("、")
    )
}

fn build_redirect_prompt(memory: &ResolvedMemory) -> String {
    if memory.province_name.is_some() && memory.subject_type.is_some() && memory.score.is_some() {
        "如果你愿意，我也可以立刻回到招生咨询，结合你的省份、科类/选科和分数继续帮你筛专业或评估具体专业录取概率。".to_owned()
    } else {
        "如果你愿意，也可以告诉我省份、科类/选科、分数、位次和意向专业，我继续帮你看录取概率或近三年分数线。".to_owned()
    }
}

fn citations_from_structured_result(result: &ChatStructuredResult) -> Vec<ChatCitation> {
    match result {
        ChatStructuredResult::ScoreQuery { records, .. } => records
            .iter()
            .take(3)
            .map(|record| ChatCitation {
                year: Some(record.year),
                source_label: record.source_label.clone(),
                source_url: record.source_url.clone(),
            })
            .collect(),
        ChatStructuredResult::ProvinceMajorList { majors, .. } => {
            let mut seen = HashSet::new();
            majors
                .iter()
                .take(3)
                .filter_map(|record| {
                    let key = format!("{}:{}", record.year, record.source_label);
                    if seen.insert(key) {
                        Some(ChatCitation {
                            year: Some(record.year),
                            source_label: record.source_label.clone(),
                            source_url: None,
                        })
                    } else {
                        None
                    }
                })
                .collect()
        }
        ChatStructuredResult::MajorProvinceList { provinces, .. } => {
            let mut seen = HashSet::new();
            provinces
                .iter()
                .take(3)
                .filter_map(|record| {
                    let key = format!("{}:{}", record.year, record.source_label);
                    if seen.insert(key) {
                        Some(ChatCitation {
                            year: Some(record.year),
                            source_label: record.source_label.clone(),
                            source_url: None,
                        })
                    } else {
                        None
                    }
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn render_province_major_list_answer(result: &ChatStructuredResult) -> String {
    render_province_major_list_answer_with_limit(result, province_major_default_display_limit())
}

fn render_province_major_list_answer_for_query(
    result: &ChatStructuredResult,
    query: &str,
) -> String {
    let limit = if asks_expanded_list(query) {
        province_major_expanded_display_limit()
    } else {
        province_major_default_display_limit()
    };
    render_province_major_list_answer_with_limit(result, limit)
}

fn render_province_major_list_answer_with_limit(
    result: &ChatStructuredResult,
    display_limit: usize,
) -> String {
    let ChatStructuredResult::ProvinceMajorList {
        province,
        subject_type,
        year,
        majors,
        note,
        ..
    } = result
    else {
        return "我已经查询了分省专业信息。".to_owned();
    };

    if majors.is_empty() {
        let subject_text = subject_type
            .as_deref()
            .map(|value| format!("{value}"))
            .unwrap_or_default();
        return format!(
            "我查了已导入的分省录取统计，暂时没有找到{province}{subject_text}对应的专业记录。这里不能直接判断为学校不在该省招生，建议以当年省级招生计划和学校招生章程为准。"
        );
    }

    let distinct_major_count = majors
        .iter()
        .map(|item| item.major_name.as_str())
        .collect::<HashSet<_>>()
        .len();
    let effective_limit = if majors.len() <= display_limit {
        majors.len()
    } else {
        display_limit
    };
    let list = majors
        .iter()
        .take(effective_limit)
        .map(|item| {
            let count = item
                .admitted_count
                .map(|value| format!("，录取{value}人"))
                .unwrap_or_default();
            let score = item
                .min_score
                .map(|value| format!("，最低分{value}"))
                .unwrap_or_default();
            format!("{}（{}{}{}）", item.major_name, item.batch, count, score)
        })
        .collect::<Vec<_>>()
        .join("、");
    let more = if majors.len() > effective_limit {
        format!(
            "我先列出前 {effective_limit} 条代表性专业/批次记录，完整记录共 {} 条",
            majors.len(),
        )
    } else {
        format!("共 {} 条专业/批次记录", majors.len())
    };
    let subject_text = subject_type
        .as_deref()
        .map(|value| format!("{value}"))
        .unwrap_or_else(|| "未区分科类".to_owned());
    let year_text = year
        .map(|value| value.to_string())
        .unwrap_or_else(|| "最新一年".to_owned());
    let continuation = if majors.len() > effective_limit {
        "如果你需要完整名单，我可以继续按“本科批、专项计划、公费师范、艺术类、体育类”等批次分段列出来，读起来会更清楚。"
    } else {
        ""
    };

    format!(
        "我查到已导入录取统计中，{province}{year_text}年（{subject_text}）有录取记录的专业名去重后约 {distinct_major_count} 个；按专业名称、批次和统计口径展开，{more}：{list}。\n\n这里同一个专业在本科批、专项计划、公费师范生、艺术类或体育类等不同批次中可能会重复出现。{continuation}\n\n需要说明：{note}如果你要填报志愿，最终仍要以所在省级招生考试机构公布的招生计划和学校官方招生章程为准。"
    )
}

fn render_major_province_list_answer(result: &ChatStructuredResult) -> String {
    let ChatStructuredResult::MajorProvinceList {
        major_name,
        subject_type,
        year,
        provinces,
        note,
        ..
    } = result
    else {
        return "我已经查询了专业覆盖省份信息。".to_owned();
    };

    if provinces.is_empty() {
        let subject_text = subject_type
            .as_deref()
            .map(|value| format!("{value}"))
            .unwrap_or_default();
        return format!(
            "我查了已导入的分省录取统计，暂时没有找到{major_name}{subject_text}对应的省份录取记录。这里不能直接判断为学校不招该专业，建议以当年省级招生计划和学校招生章程为准。"
        );
    }

    let list = provinces
        .iter()
        .take(80)
        .map(|item| {
            let count = item
                .admitted_count
                .map(|value| format!("，录取{value}人"))
                .unwrap_or_default();
            let score = item
                .min_score
                .map(|value| format!("，最低分{value}"))
                .unwrap_or_default();
            format!("{}（{}{}{}）", item.province_name, item.batch, count, score)
        })
        .collect::<Vec<_>>()
        .join("、");
    let subject_text = subject_type
        .as_deref()
        .map(|value| format!("{value}"))
        .unwrap_or_else(|| "未区分科类".to_owned());
    let year_text = year
        .map(|value| value.to_string())
        .unwrap_or_else(|| "最新一年".to_owned());

    format!(
        "我查到已导入录取统计中，{major_name}在{year_text}年（{subject_text}）有录取记录的省份/地区共 {} 个：{list}。\n\n需要说明：{note}如果你要填报志愿，最终仍要以所在省教育考试院公布的招生计划和学校官方招生章程为准。",
        provinces.len()
    )
}

fn province_major_default_display_limit() -> usize {
    std::env::var("PROVINCE_MAJOR_LIST_DEFAULT_LIMIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (8..=80).contains(value))
        .unwrap_or(36)
}

fn province_major_expanded_display_limit() -> usize {
    std::env::var("PROVINCE_MAJOR_LIST_EXPANDED_LIMIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (24..=220).contains(value))
        .unwrap_or(120)
}

fn asks_expanded_list(message: &str) -> bool {
    contains_any_text(
        message,
        &[
            "全部",
            "所有",
            "完整",
            "详细列",
            "都列",
            "全列",
            "完整名单",
            "全部名单",
            "所有专业",
            "所有省份",
        ],
    )
}

fn render_score_answer(result: &ChatStructuredResult) -> String {
    let ChatStructuredResult::ScoreQuery {
        province,
        subject_type,
        diagnostics,
        records,
        ..
    } = result
    else {
        return "我已经查询了录取统计。".to_owned();
    };

    if records.is_empty() {
        return format!(
            "我查了已导入的 2021-2025 年录取统计，暂时没有找到{province}对应专业的分专业录取记录。你可以换一个专业名称，或补充专业全称我再查。"
        );
    }
    let latest = &records[0];
    let requested_subject = diagnostics
        .as_ref()
        .and_then(|value| value.get("requestedSubjectType"))
        .and_then(|value| value.as_str());
    let subject_text = match (subject_type.as_deref(), requested_subject) {
        (Some("未区分"), Some(requested)) => {
            format!(" 未区分科类/普通类记录（你提到的{requested}未在统计表中单列） ")
        }
        (Some("未区分"), None) => " 未区分科类/普通类记录 ".to_owned(),
        (Some(value), _) => format!(" {value} "),
        (None, _) => " ".to_owned(),
    };
    format!(
        "根据已导入的历年录取统计，{}{}该专业 {} 年最低分为 {} 分。这个结果来自录取统计表，具体填报仍要结合当年招生计划和省级招生主管部门公布信息。",
        province, subject_text, latest.year, latest.min_score
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_reply_without_losing_text() {
        let reply = "可以查到。这里是第二句。";
        assert_eq!(chunk_reply_text(reply).join(""), reply);
    }

    #[test]
    fn redirect_uses_generic_slots() {
        let memory = ResolvedMemory {
            province_name: Some("河北".to_owned()),
            subject_type: Some("历史类".to_owned()),
            score: Some(500.0),
            major_name: Some("汉语言文学".to_owned()),
            ..ResolvedMemory::default()
        };
        let prompt = build_redirect_prompt(&memory);
        assert!(prompt.contains("你的省份"));
        assert!(!prompt.contains("河北"));
        assert!(!prompt.contains("汉语言文学"));
    }

    #[test]
    fn score_probability_follow_up_is_not_a_major_switch() {
        assert_eq!(extract_switch_major_query("500分能上吗？"), None);
        assert_eq!(extract_major_phrase("500分能上吗？"), None);
        assert!(asks_probability("河北500分报汉语言文学师范类有希望吗？"));
        assert!(asks_probability("这个分数录取机会大不大？"));
        assert!(asks_training_plan_context(
            "如果喜欢做实验，也想以后当生物老师，这个专业适合吗？"
        ));
        assert!(asks_training_plan_context(
            "地理科学（师范类）和行知实验班有什么区别？"
        ));
        assert!(asks_program_comparison_context(
            "计算机科学与技术师范和非师范怎么选？"
        ));
        assert!(asks_province_admission_major_list(
            "哈师大在山东招生哪些专业"
        ));
        assert!(asks_province_admission_major_list(
            "我问的是哈师大在山东招哪些专业"
        ));
        assert!(asks_province_admission_major_list("山东招什么专业？"));
        assert!(asks_major_admission_province_list(
            "物联网工程在哪些省份有招生？"
        ));
        assert!(asks_major_admission_province_list(
            "英语师范类在哪些地区有录取记录？"
        ));
    }

    #[test]
    fn training_plan_follow_up_inherits_major_from_history_user_message() {
        let mut memory = ResolvedMemory::default();
        let history = vec![ConversationMessage {
            role: "user".to_owned(),
            content: "数学与应用数学培养方案讲一下".to_owned(),
            structured_payload: None,
            citations: Vec::new(),
            created_at: None,
        }];

        enrich_memory_from_history(&mut memory, &history);

        assert_eq!(memory.major_name.as_deref(), Some("数学与应用数学"));
        assert_eq!(
            contextual_knowledge_query("主要课程呢？", &memory),
            "数学与应用数学 主要课程呢？"
        );
        assert_eq!(
            contextual_knowledge_query("音乐学专业的教育实习和实践环节怎么安排？", &memory),
            "音乐学专业的教育实习和实践环节怎么安排？"
        );
        assert_eq!(
            contextual_knowledge_query_with_history(
                "毕业要求呢？",
                &ResolvedMemory::default(),
                &history
            ),
            "数学与应用数学专业 毕业要求呢？"
        );
        assert_eq!(
            contextual_knowledge_query_with_history(
                "音乐学专业的教育实习和实践环节怎么安排？",
                &ResolvedMemory::default(),
                &history
            ),
            "音乐学专业的教育实习和实践环节怎么安排？"
        );
        assert_eq!(
            extract_major_phrase("数学与应用数学 主要课程呢？").as_deref(),
            Some("数学与应用数学")
        );
        assert_eq!(extract_major_phrase("主要课程呢？"), None);
        assert_eq!(extract_major_phrase("主要课程有哪些？"), None);
        assert_eq!(
            contextual_knowledge_query_with_history(
                "主要课程有哪些？",
                &ResolvedMemory::default(),
                &history
            ),
            "数学与应用数学专业 主要课程有哪些？"
        );
        let biology_history = vec![ConversationMessage {
            role: "user".to_owned(),
            content: "生物科学专业主要学哪些核心课程？实践环节有哪些？".to_owned(),
            structured_payload: None,
            citations: Vec::new(),
            created_at: None,
        }];
        assert_eq!(
            contextual_knowledge_query_with_history(
                "如果我喜欢做实验，也想以后当生物老师，这个专业的培养目标适合吗？",
                &ResolvedMemory::default(),
                &biology_history
            ),
            "生物科学专业 如果我喜欢做实验，也想以后当生物老师，这个专业的培养目标适合吗？"
        );
        memory.major_name = Some("数据科学与大数据技术".to_owned());
        enrich_memory_from_message(&mut memory, "数据结构和操作系统课程有没有？");
        assert_eq!(memory.major_name.as_deref(), Some("数据科学与大数据技术"));
        assert_eq!(
            contextual_knowledge_query("数据结构和操作系统课程有没有？", &memory),
            "数据科学与大数据技术 数据结构和操作系统课程有没有？"
        );
        memory.major_name = Some("地理信息科学".to_owned());
        assert_eq!(
            contextual_knowledge_query("GIS和遥感相关课程有没有？", &memory),
            "地理信息科学 GIS和遥感相关课程有没有？"
        );
        assert_eq!(
            contextual_knowledge_query("别只说课，实践环节怎么安排？", &memory),
            "地理信息科学 别只说课，实践环节怎么安排？"
        );
        memory.major_name = Some("西班牙语".to_owned());
        assert_eq!(
            contextual_knowledge_query("简单介绍一下学校", &memory),
            "简单介绍一下学校"
        );
        assert_eq!(
            contextual_knowledge_query("学校校训是什么？", &memory),
            "学校校训是什么？"
        );
        assert_eq!(
            contextual_knowledge_query_with_history(
                "学校校训是什么？",
                &memory,
                &history
            ),
            "学校校训是什么？"
        );
        assert_eq!(
            contextual_knowledge_query_with_history(
                "地理科学（师范类）和行知实验班有什么区别？",
                &memory,
                &history
            ),
            "地理科学（师范类）和行知实验班有什么区别？"
        );
        assert!(is_broad_school_or_campus_query("简单介绍一下学校"));
        assert!(is_school_level_fact_query("学校校训是什么？"));
        assert!(!is_broad_school_or_campus_query("简单介绍一下西语学院"));
        memory.major_name = Some("生物科学（师范类）".to_owned());
        assert_eq!(
            contextual_knowledge_query(
                "如果我喜欢做实验，也想以后当生物老师，这个专业的培养目标适合吗？",
                &memory
            ),
            "生物科学（师范类） 如果我喜欢做实验，也想以后当生物老师，这个专业的培养目标适合吗？"
        );
        assert_eq!(
            contextual_knowledge_query(
                "如果我喜欢做实验，也想以后当生物老师，这个专业适合吗？",
                &memory
            ),
            "生物科学（师范类） 如果我喜欢做实验，也想以后当生物老师，这个专业适合吗？ 培养目标 毕业要求 实践环节"
        );
        assert_eq!(
            extract_major_phrase("山东数据科学与大数据技术2021到2025录取线").as_deref(),
            Some("数据科学与大数据技术")
        );
        assert_eq!(extract_major_phrase("毕业条件需要多少学分？"), None);
        assert_eq!(extract_major_phrase("这个专业培养目标讲一下"), None);
        assert_eq!(
            extract_major_phrase("数据科学与大数据技术 毕业条件需要多少学分？").as_deref(),
            Some("数据科学与大数据技术")
        );
        assert_eq!(
            extract_major_phrase("音乐学专业的教育实习和实践环节怎么安排？").as_deref(),
            Some("音乐学")
        );
        assert_eq!(
            extract_major_phrase("生物科学专业主要学哪些核心课程？实践环节有哪些？").as_deref(),
            Some("生物科学")
        );
        assert_eq!(
            extract_major_phrase(
                "生物科学专业 如果我喜欢做实验，也想以后当生物老师，这个专业的培养目标适合吗？"
            )
            .as_deref(),
            Some("生物科学")
        );
        assert_eq!(
            extract_major_phrase("汉语言文学专业核心课程有哪些？").as_deref(),
            Some("汉语言文学")
        );
        assert_eq!(
            extract_major_phrase("音乐学（师范）的实践环节有哪些？").as_deref(),
            Some("音乐学（师范）")
        );
        assert_eq!(
            extract_major_phrase("音乐学（师范）的实践环节有哪些？教育实习一般多久？").as_deref(),
            Some("音乐学（师范）")
        );
        assert_eq!(
            extract_major_phrase("计算机科学与技术（师范）培养方案里有没有数据结构这些课？")
                .as_deref(),
            Some("计算机科学与技术（师范）")
        );
    }

    #[test]
    fn probability_with_contextual_score_basis_stays_probability_assessment() {
        let memory = ResolvedMemory {
            province_name: Some("河北".to_owned()),
            subject_type: Some("历史类".to_owned()),
            score: Some(500.0),
            major_name: Some("汉语言文学（师范类）".to_owned()),
            major_slug: Some("汉语言文学（师范类）".to_owned()),
            ..ResolvedMemory::default()
        };
        let plan = combined_request_plan(
            "我是河北本科批500分，想报汉语言文学（师范类），结合近三年分数看有希望吗？",
            &memory,
            &RetrievalIntent::ProbabilityAssessment,
            None,
        );

        assert!(plan.is_none());

        let explicit = combined_request_plan(
            "黑龙江理科500分数学与应用数学录取概率和录取线",
            &memory,
            &RetrievalIntent::ProbabilityAssessment,
            None,
        )
        .expect("combined score and probability plan");
        assert!(explicit.include_probability);
        assert!(explicit.include_score);
    }

    #[test]
    fn admission_score_year_inherits_only_single_supported_years_for_followups() {
        assert_eq!(
            extract_supported_admission_year("2025年多少分能进计算机科学与技术专业？"),
            Some(2025)
        );
        assert_eq!(
            extract_supported_admission_year("2021到2025年汉语言文学录取线是多少？"),
            None
        );
        assert_eq!(
            extract_supported_admission_year("2021年到2025年汉语言文学录取线是多少？"),
            None
        );

        let history = vec![ConversationMessage {
            role: "user".to_owned(),
            content: "2025年多少分能进哈师大计算机科学与技术专业？".to_owned(),
            structured_payload: None,
            citations: Vec::new(),
            created_at: None,
        }];
        assert_eq!(
            resolve_admission_score_year_for_turn("河北", &history, &RetrievalIntent::ScoreQuery),
            Some(2025)
        );

        let range_history = vec![ConversationMessage {
            role: "user".to_owned(),
            content: "黑龙江物理类计算机科学与技术2021到2025录取线是多少？".to_owned(),
            structured_payload: None,
            citations: Vec::new(),
            created_at: None,
        }];
        assert_eq!(
            resolve_admission_score_year_for_turn(
                "河北",
                &range_history,
                &RetrievalIntent::ScoreQuery
            ),
            None
        );
    }

    #[test]
    fn knowledge_memory_can_refine_major_variant() {
        let mut memory = ResolvedMemory {
            major_name: Some("计算机科学与技术".to_owned()),
            major_slug: Some("计算机科学与技术".to_owned()),
            ..ResolvedMemory::default()
        };
        let structured = ChatStructuredResult::KnowledgeAnswer {
            query: "计算机科学与技术（师范）培养方案里有没有数据结构？".to_owned(),
            faq: Vec::new(),
            policies: Vec::new(),
            vector_chunks: vec![domain::VectorChunkEvidence {
                id: "chunk-1".to_owned(),
                title: None,
                content: "专业核心课程包括数据结构。".to_owned(),
                category: Some("培养方案".to_owned()),
                year: Some(2025),
                similarity: Some(0.9),
                metadata: serde_json::json!({
                    "majorName": "计算机科学与技术专业（师范）",
                    "documentKind": "training_plan"
                }),
            }],
        };

        merge_memory_from_structured(&mut memory, &structured);

        assert_eq!(
            memory.major_name.as_deref(),
            Some("计算机科学与技术专业（师范）")
        );
    }

    #[test]
    fn score_comparison_uses_context_major_and_explicit_comparison_major() {
        let memory = ResolvedMemory {
            province_name: Some("河北".to_owned()),
            major_name: Some("地理科学（师范类）".to_owned()),
            major_slug: Some("地理科学（师范类）".to_owned()),
            ..ResolvedMemory::default()
        };
        let candidates = vec![
            domain::MajorCandidate {
                slug: "地理信息科学".to_owned(),
                name: "地理信息科学".to_owned(),
                code: None,
                is_normal_major: false,
                latest_score: None,
            },
            domain::MajorCandidate {
                slug: "人文地理与城乡规划".to_owned(),
                name: "人文地理与城乡规划".to_owned(),
                code: None,
                is_normal_major: false,
                latest_score: None,
            },
        ];
        let resolved = resolve_score_comparison_candidates(
            "那它和地理信息科学相比，最近三年哪个分数更高一点？",
            &memory,
            candidates,
        );
        let names = resolved
            .iter()
            .map(|candidate| candidate.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["地理科学（师范类）", "地理信息科学"]);
    }

    #[test]
    fn admission_policy_query_handles_same_score_wording() {
        assert!(is_admission_policy_query(
            "如果两个考生投档成绩一样，学校会优先录取谁？"
        ));
        assert!(is_admission_policy_query("分数相同的时候怎么录取？"));
    }

    #[test]
    fn high_risk_fact_sanitizer_replaces_unverified_admissions_phone() {
        let structured = ChatStructuredResult::GeneralAnswer {
            answer: "普通咨询".to_owned(),
            redirect_prompt: String::new(),
            collected_profile: ResolvedMemory::default(),
        };
        let reply =
            sanitize_high_risk_facts("招生咨询电话是0451-88060176。".to_owned(), &structured);
        assert!(reply.contains("0451-88067377"));
        assert!(!reply.contains("0451-88060176"));
    }

    #[test]
    fn high_risk_fact_sanitizer_removes_unverified_art_formula() {
        let structured = ChatStructuredResult::KnowledgeAnswer {
            query: "艺术类怎么录取？".to_owned(),
            faq: Vec::new(),
            policies: Vec::new(),
            vector_chunks: Vec::new(),
        };
        let reply = sanitize_high_risk_facts(
            "美术类按文化课×40%+专业课×60%录取。其他规则以省招办为准。".to_owned(),
            &structured,
        );
        assert!(!reply.contains("40%"));
        assert!(!reply.contains("60%"));
        assert!(reply.contains("其他规则以省招办为准"));

        let reply = sanitize_high_risk_facts(
            "艺术类在部分省份按综合分投档，不能直接写成文化课成绩×40% + 美术类专业统考成绩×60%。"
                .to_owned(),
            &structured,
        );
        assert!(!reply.contains("40%"));
        assert!(!reply.contains("60%"));
    }

    #[test]
    fn high_risk_fact_sanitizer_normalizes_evidence_backed_program_typos() {
        let structured = ChatStructuredResult::KnowledgeAnswer {
            query: "地理科学（师范类）和行知班有什么区别？".to_owned(),
            faq: Vec::new(),
            policies: Vec::new(),
            vector_chunks: Vec::new(),
        };
        let reply = sanitize_high_risk_facts(
            "行知班更强调科研训练，而言行班也有更高毕业要求。".to_owned(),
            &structured,
        );
        assert!(reply.contains("行知班也有更高毕业要求"));
        assert!(!reply.contains("言行班"));
    }

    #[test]
    fn probability_uses_actual_unspecified_subject_when_requested_subject_has_no_separate_records()
    {
        let memory = ResolvedMemory {
            province_name: Some("天津".to_owned()),
            subject_type: Some("历史类".to_owned()),
            score: Some(540.0),
            major_name: Some("学前教育（师范类）".to_owned()),
            ..ResolvedMemory::default()
        };
        let score_records = ChatStructuredResult::ScoreQuery {
            major_name: "学前教育（师范类）".to_owned(),
            province: "天津".to_owned(),
            subject_type: Some("历史类".to_owned()),
            records: vec![domain::AdmissionScoreRecord {
                year: 2025,
                batch: "本科批".to_owned(),
                subject_type: "未区分".to_owned(),
                admitted_count: Some(5),
                min_score: 528,
                avg_score: None,
                max_score: None,
                min_rank: Some(32064),
                source_label: "录取统计表".to_owned(),
                source_url: None,
            }],
            summary: domain::ScoreSummary {
                total_records: 1,
                years: vec![2025],
                source_labels: vec!["录取统计表".to_owned()],
            },
            diagnostics: None,
        };

        let structured = build_probability_from_memory(&memory, &score_records);
        let ChatStructuredResult::ProbabilityAssessment { assessment } = &structured else {
            panic!("expected probability assessment");
        };
        assert_eq!(
            assessment.get("subjectType").and_then(|v| v.as_str()),
            Some("未区分")
        );
        assert_eq!(
            assessment
                .get("requestedSubjectType")
                .and_then(|v| v.as_str()),
            Some("历史类")
        );
        let reply = render_probability_answer(&structured);
        assert!(reply.contains("未区分科类/普通类记录"));
        assert!(reply.contains("历史类未在统计表中单列"));
    }

    #[test]
    fn finalizer_removes_false_negative_for_unspecified_subject_probability() {
        let structured = ChatStructuredResult::ProbabilityAssessment {
            assessment: json!({
                "province": "天津",
                "subjectType": "未区分",
                "requestedSubjectType": "历史类",
                "score": 540.0,
                "major": "学前教育（师范类）",
                "probability": 64,
                "level": "medium",
                "confidence": "medium",
                "summary": "比最近一年最低分高12分。",
                "factors": [],
                "scoreSummary": {
                    "recordCount": 1,
                    "latestYear": 2025,
                    "latestMinScore": 528,
                    "records": []
                }
            }),
        };

        let reply = finalize_reply(
            "暂未找到天津历史类学前教育（师范类）专业的具体录取分数。".to_owned(),
            &structured,
            &ResolvedMemory::default(),
        );
        assert!(!reply.contains("暂未找到"));
        assert!(reply.contains("未区分科类/普通类记录"));
        assert!(reply.contains("528"));
    }

    #[test]
    fn finalizer_requires_unspecified_subject_boundary_for_probability() {
        let structured = ChatStructuredResult::ProbabilityAssessment {
            assessment: json!({
                "province": "天津",
                "subjectType": "未区分",
                "score": 540.0,
                "major": "学前教育（师范类）",
                "probability": 64,
                "level": "medium",
                "confidence": "medium",
                "summary": "比最近一年最低分高12分。",
                "factors": [],
                "scoreSummary": {
                    "recordCount": 1,
                    "latestYear": 2025,
                    "latestMinScore": 528,
                    "records": []
                }
            }),
        };

        let reply = finalize_reply(
            "540分有希望，参考概率约64%。".to_owned(),
            &structured,
            &ResolvedMemory::default(),
        );
        assert!(reply.contains("未区分科类/普通类记录"));
        assert!(reply.contains("528"));
    }

    #[test]
    fn finalizer_preserves_unspecified_subject_boundary_for_score_query() {
        let structured = ChatStructuredResult::ScoreQuery {
            major_name: "汉语言文学（师范类）".to_owned(),
            province: "山东".to_owned(),
            subject_type: Some("未区分".to_owned()),
            records: vec![domain::AdmissionScoreRecord {
                year: 2025,
                batch: "本科批".to_owned(),
                subject_type: "未区分".to_owned(),
                admitted_count: Some(4),
                min_score: 565,
                avg_score: None,
                max_score: None,
                min_rank: Some(60437),
                source_label: "录取统计表".to_owned(),
                source_url: None,
            }],
            summary: domain::ScoreSummary {
                total_records: 1,
                years: vec![2025],
                source_labels: vec!["录取统计表".to_owned()],
            },
            diagnostics: Some(json!({
                "requestedSubjectType": "历史类",
                "actualSubjectType": "未区分"
            })),
        };

        let reply = finalize_reply(
            "这份数据仅覆盖山东历史类考生。".to_owned(),
            &structured,
            &ResolvedMemory::default(),
        );
        assert!(reply.contains("未区分科类/普通类记录"));
        assert!(reply.contains("历史类未在统计表中单列"));
        assert!(reply.contains("565"));
    }

    #[test]
    fn finalizer_keeps_combined_score_and_probability_when_bundle_needs_subject_boundary() {
        let score_query = ChatStructuredResult::ScoreQuery {
            major_name: "数学与应用数学（师范类）".to_owned(),
            province: "黑龙江".to_owned(),
            subject_type: Some("未区分".to_owned()),
            records: vec![domain::AdmissionScoreRecord {
                year: 2025,
                batch: "本科批".to_owned(),
                subject_type: "未区分".to_owned(),
                admitted_count: Some(120),
                min_score: 530,
                avg_score: None,
                max_score: None,
                min_rank: None,
                source_label: "录取统计表".to_owned(),
                source_url: None,
            }],
            summary: domain::ScoreSummary {
                total_records: 1,
                years: vec![2025],
                source_labels: vec!["录取统计表".to_owned()],
            },
            diagnostics: Some(json!({
                "requestedSubjectType": "理科",
                "actualSubjectType": "未区分"
            })),
        };
        let probability = ChatStructuredResult::ProbabilityAssessment {
            assessment: json!({
                "province": "黑龙江",
                "subjectType": "未区分",
                "requestedSubjectType": "理科",
                "score": 500.0,
                "major": "数学与应用数学（师范类）",
                "probability": 5,
                "level": "low",
                "confidence": "low",
                "summary": "低于最近一年最低分。",
                "factors": [],
                "scoreSummary": {
                    "recordCount": 1,
                    "latestYear": 2025,
                    "latestMinScore": 530,
                    "records": []
                }
            }),
        };
        let bundle = ChatStructuredResult::EvidenceBundle {
            message: "黑龙江理科500分数学与应用数学录取概率和录取线".to_owned(),
            results: vec![score_query, probability],
        };

        let reply = finalize_reply(
            "暂未找到黑龙江理科数学与应用数学的录取记录。".to_owned(),
            &bundle,
            &ResolvedMemory::default(),
        );
        assert!(reply.contains("530"));
        assert!(reply.contains("500分"));
        assert!(reply.contains("未区分科类/普通类记录"));
        assert!(reply.contains("参考概率"));
    }
}
