//! Atomic, idempotent publication of an evidence-backed summary (issue #54).
//!
//! `derive_summary` runs the pipeline for a set of source memories:
//! summarizer -> gate -> promotion -> one transaction that revalidates every
//! source version, writes the derivation, summary, claims, evidence and history,
//! archives the sources (they are kept, not deleted) and fixes the stream
//! counter. Any failure before COMMIT leaves sources and counters untouched.
//!
//! Child module of `surreal` so it can use the storage internals directly.

use super::*;
use crate::compression::contract::{CONTRACT_VERSION, SourceMemory, SourceOrigin};
use crate::compression::gate::{self, ValidatedClaim};
use crate::compression::lineage;
use crate::compression::{FaultPoint, SummarizeError};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

/// Where a stream-bound derivation belongs, so its counter can be fixed in the
/// same transaction.
pub(crate) struct StreamRef {
    pub id: RecordId,
}

pub(crate) struct DerivationRequest {
    pub sources: Vec<Memory>,
    pub stream: Option<StreamRef>,
    /// Category tag on the summary: `auto_summary` or `compressed`.
    pub label: &'static str,
}

pub(crate) enum DerivationOutcome {
    /// Boxed: a `Memory` is much larger than the `Deferred` reason.
    Published(Box<Memory>),
    /// Nothing was changed; the reason is a short machine-readable label.
    Deferred(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct DerivationRow {
    key: String,
    task_stream: Option<RecordId>,
    user_id: Option<String>,
    agent_id: Option<String>,
    method: String,
    algorithm: String,
    model: Option<String>,
    prompt_version: Option<String>,
    config_version: String,
    contract_version: u32,
    input_hash: String,
    source_count: u32,
    tokens_before: u64,
    tokens_after: u64,
    outcome: String,
    reason: Option<String>,
    summary_memory: Option<RecordId>,
    generated_at: Datetime,
    created_at: Datetime,
}

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct ClaimRow {
    id: RecordId,
    derivation: RecordId,
    summary_memory: Option<RecordId>,
    kind: String,
    text: String,
    status: String,
    reason: Option<String>,
    evidence_strength: Option<String>,
    corroboration: u32,
    applicability: Option<serde_json::Value>,
    qualifiers: Option<serde_json::Value>,
    user_id: Option<String>,
    agent_id: Option<String>,
    session_id: Option<String>,
    observed_at: Option<Datetime>,
    recorded_at: Datetime,
    last_verified_at: Option<Datetime>,
    valid_until: Option<Datetime>,
    superseded_by: Option<RecordId>,
    created_at: Datetime,
    updated_at: Datetime,
}

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct EvidenceRow {
    claim: RecordId,
    source: RecordId,
    source_version: u32,
    quote: String,
    origin: String,
    root_source: RecordId,
    created_at: Datetime,
}

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct HistoryRow {
    memory_id: RecordId,
    version: u32,
    old_content: Option<String>,
    new_content: String,
    changed_at: Datetime,
    change_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct ExpectedSource {
    id: RecordId,
    version: u32,
}

/// The one owner a derivation may belong to. A summary can never be visible to
/// someone a source was not visible to, so mixed owners defer.
struct CommonScope {
    scope: crate::memory::MemoryScope,
    user_id: Option<String>,
    agent_id: Option<String>,
    session_id: Option<String>,
}

fn common_scope(sources: &[Memory]) -> Option<CommonScope> {
    let first = sources.first()?;
    let same = sources.iter().all(|m| {
        m.scope == first.scope
            && m.user_id == first.user_id
            && m.agent_id == first.agent_id
            && m.session_id == first.session_id
    });
    same.then(|| CommonScope {
        scope: first.scope.clone(),
        user_id: first.user_id.clone(),
        agent_id: first.agent_id.clone(),
        session_id: first.session_id.clone(),
    })
}

fn utc(value: Datetime) -> DateTime<Utc> {
    value.into()
}

fn source_memory(memory: &Memory) -> Option<SourceMemory> {
    let id = memory.id.as_ref()?;
    Some(SourceMemory {
        id: SurrealStorage::record_id_to_string(id),
        version: memory.version,
        content: memory.content.clone(),
        origin: SourceOrigin::from_categories(&memory.categories),
        root_sources: lineage::roots_from_metadata(&memory.metadata),
        observed_at: Some(utc(memory.created_at)),
        valid_until: memory.valid_until.map(utc),
    })
}

/// Idempotency key: the same stream, label, source versions and summarizer
/// configuration always produce the same key, so a retry cannot duplicate.
fn derivation_key(
    label: &str,
    stream: Option<&RecordId>,
    sources: &[SourceMemory],
    descriptor: &crate::compression::SummarizerDescriptor,
) -> String {
    let mut ordered: Vec<String> = sources
        .iter()
        .map(|s| format!("{}@{}", s.id, s.version))
        .collect();
    ordered.sort();
    let mut hasher = Sha256::new();
    for part in [
        label.to_string(),
        stream
            .map(SurrealStorage::record_id_to_string)
            .unwrap_or_default(),
        CONTRACT_VERSION.to_string(),
        descriptor.algorithm.clone(),
        descriptor.model.clone().unwrap_or_default(),
        descriptor.config_version.clone(),
    ]
    .iter()
    .chain(ordered.iter())
    {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn summarize_error_label(error: &SummarizeError) -> &'static str {
    match error {
        SummarizeError::Unavailable(_) => "summarizer_unavailable",
        SummarizeError::Timeout => "summarizer_timeout",
        SummarizeError::Malformed(_) => "summarizer_malformed_output",
        SummarizeError::InputTooLarge => "summarizer_input_too_large",
        SummarizeError::Provider(_) => "summarizer_failed",
    }
}

fn record_from_id_string(id: &str) -> Option<RecordId> {
    let (table, key) = id.split_once(':')?;
    Some(RecordId::new(table, key.to_string()))
}

fn json_of<T: Serialize>(value: &T) -> Option<serde_json::Value> {
    serde_json::to_value(value).ok()
}

impl SurrealStorage {
    /// Run the compression pipeline for `request` and publish the result
    /// atomically. Returns `Deferred` (changing nothing) whenever the output
    /// cannot be safely produced or validated.
    pub(crate) async fn derive_summary(
        &self,
        request: DerivationRequest,
    ) -> Result<DerivationOutcome> {
        let runtime = &self.compression;
        let deferred = |reason: &str| {
            tracing::info!(reason, "compression deferred; sources left intact");
            Ok(DerivationOutcome::Deferred(reason.to_string()))
        };

        let Some(owner) = common_scope(&request.sources) else {
            return deferred("mixed_or_empty_scope");
        };
        let sources: Vec<SourceMemory> = request.sources.iter().filter_map(source_memory).collect();
        if sources.len() != request.sources.len() {
            return deferred("source_without_id");
        }

        // Generation
        let descriptor = runtime.summarizer.descriptor();
        let candidates = match tokio::time::timeout(
            runtime.summarizer_timeout,
            runtime.summarizer.extract(&sources),
        )
        .await
        {
            Ok(Ok(candidates)) => candidates,
            Ok(Err(error)) => return deferred(summarize_error_label(&error)),
            Err(_) => return deferred("summarizer_timeout"),
        };
        if runtime.fault(FaultPoint::Generation) {
            anyhow::bail!("injected fault at generation");
        }

        // Validation
        let gated = match gate::run(&sources, candidates, &runtime.limits, descriptor.method) {
            Ok(gated) => gated,
            Err(reason) => return deferred(&reason.to_string()),
        };
        if runtime.fault(FaultPoint::Validation) {
            anyhow::bail!("injected fault at validation");
        }

        let stream_id = request.stream.as_ref().map(|s| s.id.clone());
        let key = derivation_key(request.label, stream_id.as_ref(), &sources, &descriptor);
        if let Some(existing) = self.published_derivation(&key).await? {
            return Ok(DerivationOutcome::Published(Box::new(existing)));
        }

        let embedding = match self.embedding_service.embed(&gated.rendered).await {
            Ok(embedding) => embedding,
            Err(_) => return deferred("embedding_failed"),
        };

        // Build every row up front so the transaction is a single resubmittable
        // unit with stable keys.
        let now = Datetime::from(runtime.clock.now());
        let derivation_id = RecordId::new("memory_derivation", key.clone());
        let summary_id = RecordId::new("memory", Uuid::new_v4().to_string());

        let all_sources: Vec<&SourceMemory> = sources.iter().collect();
        let roots = lineage::independent_roots(&all_sources);
        let mut summary = Memory::new(
            gated.rendered.clone(),
            owner.user_id.clone(),
            owner.agent_id.clone(),
            owner.session_id.clone(),
            vec![
                request.label.to_string(),
                format!("method:{}", descriptor.method.as_str()),
            ],
        );
        summary.scope = owner.scope.clone();
        summary.memory_type = crate::memory::MemoryType::Semantic;
        summary.task_stream_id = stream_id.clone();
        summary.token_count = Some(gated.tokens_after as u32);
        summary.embedding = Some(embedding);
        summary.importance = request
            .sources
            .iter()
            .map(|m| m.importance)
            .fold(0.0, f32::max);
        summary.valid_until = request
            .sources
            .iter()
            .filter_map(|m| m.valid_until)
            .min_by_key(|d| utc(*d));
        summary.metadata = Some(lineage::summary_metadata(
            &key,
            descriptor.method.as_str(),
            &roots,
            CONTRACT_VERSION,
        ));
        summary.created_at = now;
        summary.updated_at = now;
        summary.version = 1;
        let summary_content = summary.content.clone();
        let summary_tokens = gated.tokens_after;
        let db_summary = DbMemory::from(summary);

        let (claims, evidence) = claim_rows(
            &gated.claims,
            ClaimContext {
                sources: &sources,
                memories: &request.sources,
                key: &key,
                derivation_id: &derivation_id,
                summary_id: &summary_id,
                owner: &owner,
                now,
            },
        );
        let archived_tokens: u64 = request
            .sources
            .iter()
            .map(|m| {
                m.token_count
                    .map(u64::from)
                    .unwrap_or_else(|| Self::estimate_tokens(&m.content) as u64)
            })
            .sum();
        let source_ids: Vec<RecordId> = request
            .sources
            .iter()
            .filter_map(|m| m.id.clone())
            .collect();
        let expected: Vec<ExpectedSource> = request
            .sources
            .iter()
            .filter_map(|m| {
                m.id.clone().map(|id| ExpectedSource {
                    id,
                    version: m.version,
                })
            })
            .collect();
        let mut history: Vec<HistoryRow> = request
            .sources
            .iter()
            .filter_map(|m| {
                m.id.clone().map(|id| HistoryRow {
                    memory_id: id,
                    version: m.version + 1,
                    old_content: Some(m.content.clone()),
                    new_content: m.content.clone(),
                    changed_at: now,
                    change_type: "summarized".to_string(),
                })
            })
            .collect();
        history.push(HistoryRow {
            memory_id: summary_id.clone(),
            version: 1,
            old_content: None,
            new_content: summary_content,
            changed_at: now,
            change_type: "created".to_string(),
        });
        let derivation = DerivationRow {
            key: key.clone(),
            task_stream: stream_id.clone(),
            user_id: owner.user_id.clone(),
            agent_id: owner.agent_id.clone(),
            method: descriptor.method.as_str().to_string(),
            algorithm: descriptor.algorithm.clone(),
            model: descriptor.model.clone(),
            prompt_version: descriptor.prompt_version.clone(),
            config_version: descriptor.config_version.clone(),
            contract_version: CONTRACT_VERSION,
            input_hash: crate::compression::text::content_hash(
                &sources
                    .iter()
                    .map(|s| s.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            source_count: sources.len() as u32,
            tokens_before: gated.tokens_before,
            tokens_after: summary_tokens,
            outcome: "published".to_string(),
            reason: None,
            summary_memory: Some(summary_id.clone()),
            generated_at: now,
            created_at: now,
        };

        // Insertion + activation: one transaction.
        let mut txn = String::from(
            "BEGIN TRANSACTION;\n\
             FOR $e IN $expected {\n\
               LET $cur = (SELECT version, status FROM ONLY $e.id);\n\
               IF $cur = NONE OR $cur.version != $e.version \
                  OR ($cur.status != NONE AND $cur.status != 'active') { THROW 'source_drift'; };\n\
             };\n\
             CREATE ONLY $derivation_id CONTENT $derivation;\n\
             CREATE ONLY $summary_id CONTENT $summary;\n\
             INSERT INTO memory_history $history;\n\
             INSERT INTO memory_claim $claims;\n\
             INSERT INTO memory_claim_evidence $evidence;\n\
             UPDATE memory SET status = 'summarized', derived_into = $derivation_id, \
               version += 1, updated_at = $now WHERE id IN $source_ids;\n",
        );
        if stream_id.is_some() {
            txn.push_str(
                "UPDATE task_stream SET \
                   total_tokens = math::max([0, total_tokens - $archived]) + $summary_tokens, \
                   summary_count += 1, last_active = $now WHERE id = $stream_id;\n",
            );
        }
        txn.push_str("IF $inject_fault { THROW 'injected_fault'; };\nCOMMIT TRANSACTION;");

        if runtime.fault(FaultPoint::Insertion) {
            anyhow::bail!("injected fault before insertion");
        }
        let inject_fault = runtime.fault(FaultPoint::Activation);
        let db = self.live_db()?;
        let result = self
            .write_retrying_conflicts("derive_summary", || async {
                let mut query = db
                    .query(txn.clone())
                    .bind(("expected", expected.clone()))
                    .bind(("derivation_id", derivation_id.clone()))
                    .bind(("derivation", derivation.clone()))
                    .bind(("summary_id", summary_id.clone()))
                    .bind(("summary", db_summary.clone()))
                    .bind(("history", history.clone()))
                    .bind(("claims", claims.clone()))
                    .bind(("evidence", evidence.clone()))
                    .bind(("source_ids", source_ids.clone()))
                    .bind(("now", now))
                    .bind(("inject_fault", inject_fault));
                if let Some(id) = &stream_id {
                    query = query
                        .bind(("stream_id", id.clone()))
                        .bind(("archived", archived_tokens))
                        .bind(("summary_tokens", summary_tokens));
                }
                check_statements(query.await?)
            })
            .await;

        match result {
            Ok(_) => match self.published_derivation(&key).await? {
                Some(published) => Ok(DerivationOutcome::Published(Box::new(published))),
                None => {
                    anyhow::bail!("derivation committed but its summary could not be read back")
                }
            },
            Err(error) => {
                let typed = error.downcast_ref::<surrealdb::Error>();
                if typed.is_some_and(|e| e.is_connection()) {
                    return Err(error);
                }
                // A concurrent or earlier attempt may already have published
                // this exact derivation: that is success, not failure.
                if let Some(published) = self.published_derivation(&key).await? {
                    return Ok(DerivationOutcome::Published(Box::new(published)));
                }
                if typed.is_some_and(|e| e.is_thrown()) {
                    return deferred("source_changed");
                }
                Err(error)
            }
        }
    }

    /// Claims (with their evidence) recorded for the summary `summary_id`
    /// (`memory:<key>`). Empty for a memory that is not a summary, including
    /// every legacy memory: trust is never invented.
    pub async fn claims_for_summary(
        &self,
        summary_id: &str,
    ) -> Result<Vec<crate::compression::ClaimRecord>> {
        use crate::compression::{ClaimRecord, EvidenceRecord};
        let Some(summary) = record_from_id_string(summary_id) else {
            return Ok(Vec::new());
        };
        let db = self.live_db()?;
        let claims: Vec<ClaimRow> = db
            .query("SELECT * FROM memory_claim WHERE summary_memory = $sid ORDER BY id")
            .bind(("sid", summary))
            .await?
            .take(0)
            .unwrap_or_default();
        let ids: Vec<RecordId> = claims.iter().map(|c| c.id.clone()).collect();
        let evidence: Vec<EvidenceRow> = db
            .query("SELECT * FROM memory_claim_evidence WHERE claim IN $ids")
            .bind(("ids", ids))
            .await?
            .take(0)
            .unwrap_or_default();
        Ok(claims
            .into_iter()
            .map(|claim| ClaimRecord {
                evidence: evidence
                    .iter()
                    .filter(|e| e.claim == claim.id)
                    .map(|e| EvidenceRecord {
                        source_id: Self::record_id_to_string(&e.source),
                        source_version: e.source_version,
                        quote: e.quote.clone(),
                        origin: e.origin.clone(),
                        root_source: Self::record_id_to_string(&e.root_source),
                    })
                    .collect(),
                id: Self::record_id_to_string(&claim.id),
                kind: claim.kind,
                text: claim.text,
                status: claim.status,
                reason: claim.reason,
                evidence_strength: claim.evidence_strength,
                corroboration: claim.corroboration,
            })
            .collect())
    }

    /// The summary of an already-published derivation with this key.
    async fn published_derivation(&self, key: &str) -> Result<Option<Memory>> {
        let db = self.live_db()?;
        let rows: Vec<DerivationSummaryRef> = db
            .query("SELECT summary_memory FROM memory_derivation WHERE key = $key AND outcome = 'published'")
            .bind(("key", key.to_string()))
            .await?
            .take(0)
            .unwrap_or_default();
        let Some(summary_id) = rows.into_iter().find_map(|row| row.summary_memory) else {
            return Ok(None);
        };
        let fetched: Vec<DbMemory> = db
            .query("SELECT * FROM memory WHERE id = $id")
            .bind(("id", summary_id))
            .await?
            .take(0)
            .unwrap_or_default();
        fetched
            .into_iter()
            .next()
            .map(Self::decode_memory)
            .transpose()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct DerivationSummaryRef {
    summary_memory: Option<RecordId>,
}

/// What `claim_rows` needs besides the claims: the frozen sources and the
/// identity of the derivation being published.
#[derive(Clone, Copy)]
struct ClaimContext<'a> {
    sources: &'a [SourceMemory],
    memories: &'a [Memory],
    key: &'a str,
    derivation_id: &'a RecordId,
    summary_id: &'a RecordId,
    owner: &'a CommonScope,
    now: Datetime,
}

fn claim_rows(
    claims: &[ValidatedClaim],
    ctx: ClaimContext<'_>,
) -> (Vec<ClaimRow>, Vec<EvidenceRow>) {
    let ClaimContext {
        sources,
        memories,
        key,
        derivation_id,
        summary_id,
        owner,
        now,
    } = ctx;
    let record_of = |source_id: &str| {
        memories.iter().find_map(|m| {
            m.id.as_ref()
                .filter(|id| SurrealStorage::record_id_to_string(id) == source_id)
                .cloned()
        })
    };
    let source_of = |source_id: &str| sources.iter().find(|s| s.id == source_id);

    let mut claim_rows = Vec::with_capacity(claims.len());
    let mut evidence_rows = Vec::new();
    for (index, claim) in claims.iter().enumerate() {
        let claim_id = RecordId::new("memory_claim", format!("{key}-{index}"));
        let evidence_sources: Vec<&SourceMemory> = claim
            .candidate
            .evidence
            .iter()
            .filter_map(|e| source_of(&e.source_id))
            .collect();
        let observed = evidence_sources.iter().filter_map(|s| s.observed_at).min();
        let expiry = evidence_sources.iter().filter_map(|s| s.valid_until).min();
        claim_rows.push(ClaimRow {
            id: claim_id.clone(),
            derivation: derivation_id.clone(),
            summary_memory: Some(summary_id.clone()),
            kind: claim.candidate.kind.as_str().to_string(),
            text: claim.candidate.text.clone(),
            status: claim.promotion.status.as_str().to_string(),
            reason: Some(claim.promotion.reason.to_string()),
            evidence_strength: Some(claim.promotion.strength.label()),
            corroboration: claim.roots.len() as u32,
            applicability: json_of(&claim.candidate.applicability),
            qualifiers: json_of(&claim.candidate.qualifiers),
            user_id: owner.user_id.clone(),
            agent_id: owner.agent_id.clone(),
            session_id: owner.session_id.clone(),
            observed_at: observed.map(Datetime::from),
            recorded_at: now,
            // Unknown stays unknown: compression never verifies anything.
            last_verified_at: None,
            valid_until: expiry.map(Datetime::from),
            superseded_by: None,
            created_at: now,
            updated_at: now,
        });
        for evidence in &claim.candidate.evidence {
            let (Some(source), Some(origin)) = (
                record_of(&evidence.source_id),
                source_of(&evidence.source_id),
            ) else {
                continue;
            };
            let roots = if origin.root_sources.is_empty() {
                vec![evidence.source_id.clone()]
            } else {
                origin.root_sources.clone()
            };
            for root in roots.iter().filter_map(|r| record_from_id_string(r)) {
                evidence_rows.push(EvidenceRow {
                    claim: claim_id.clone(),
                    source: source.clone(),
                    source_version: evidence.source_version,
                    quote: evidence.quote.clone(),
                    origin: origin.origin.as_str().to_string(),
                    root_source: root,
                    created_at: now,
                });
            }
        }
    }
    (claim_rows, evidence_rows)
}
