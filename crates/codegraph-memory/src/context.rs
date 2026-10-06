// ABOUTME: Bounded historical context packing with explicit citations and review warnings.
// ABOUTME: Complete serialized wrappers count against the consuming model budget.
use crate::types::*;
use anyhow::{Result, ensure};

pub trait TokenCounter: Send + Sync {
    fn count(&self, text: &str) -> usize;
    fn exact(&self) -> bool;
}
/// Deliberately conservative fallback: one token per UTF-8 byte, including wrappers.
pub struct ConservativeCounter;
impl TokenCounter for ConservativeCounter {
    fn count(&self, text: &str) -> usize {
        text.len()
    }
    fn exact(&self) -> bool {
        false
    }
}
struct HfCounter(tokenizers::Tokenizer);
impl TokenCounter for HfCounter {
    fn count(&self, text: &str) -> usize {
        self.0.encode(text, true).map_or(text.len(), |v| v.len())
    }
    fn exact(&self) -> bool {
        true
    }
}
struct OpenAiCounter(&'static tiktoken_rs::CoreBPE);
impl TokenCounter for OpenAiCounter {
    fn count(&self, text: &str) -> usize {
        self.0.encode_with_special_tokens(text).len()
    }
    fn exact(&self) -> bool {
        true
    }
}
pub fn consuming_counter(model: Option<&str>) -> Result<Box<dyn TokenCounter>> {
    if let Some(path) = std::env::var_os("CODEGRAPH_CONTEXT_TOKENIZER_PATH") {
        let mut tokenizer = tokenizers::Tokenizer::from_file(path)
            .map_err(|e| anyhow::anyhow!("Invalid consuming-model tokenizer: {e}"))?;
        tokenizer
            .with_truncation(None)
            .map_err(|e| anyhow::anyhow!("Invalid tokenizer policy: {e}"))?;
        tokenizer.with_padding(None);
        return Ok(Box::new(HfCounter(tokenizer)));
    }
    if let Some(model) = model
        && let Ok(tokenizer) = tiktoken_rs::bpe_for_model(model)
    {
        return Ok(Box::new(OpenAiCounter(tokenizer)));
    }
    Ok(Box::new(ConservativeCounter))
}
pub fn pack(
    entries: Vec<MemoryEntry>,
    limit: usize,
    budget: usize,
    counter: &dyn TokenCounter,
) -> Result<MemoryContext> {
    pack_with_metadata(entries, limit, budget, counter, MemoryContext::default())
}
fn packed_cost(context: &MemoryContext, counter: &dyn TokenCounter) -> Result<usize> {
    let mut reserved = context.clone();
    // The selected answer may cite every delivered claim; those references belong to
    // the same response budget even though citations are collected after reasoning.
    reserved.cited_memory_refs = reserved.retrieved_memory_refs.clone();
    Ok(counter.count(&serde_json::to_string(&reserved)?) + 32)
}
pub fn pack_with_metadata(
    mut entries: Vec<MemoryEntry>,
    limit: usize,
    budget: usize,
    counter: &dyn TokenCounter,
    mut base: MemoryContext,
) -> Result<MemoryContext> {
    ensure!(
        budget > 0 && limit > 0,
        "Memory limit and token budget must be positive"
    );
    let original_status = base.status.clone();
    let partial = base.status == "partial";
    base.status = "ok".into();
    base.memories.clear();
    base.needs_verification.clear();
    base.retrieved_memory_refs.clear();
    base.cited_memory_refs.clear();
    base.estimated_tokens = 0;
    base.limit = limit;
    base.token_budget = budget;
    let mut pack = base;
    if !counter.exact() {
        pack.warnings.push(
            "Consuming-model tokenizer unavailable; using conservative UTF-8 byte estimate".into(),
        );
    }
    for entry in &mut entries {
        entry.memory.vectors.clear();
        for anchor in &mut entry.memory.anchors {
            anchor.historical_snippet.clear();
        }
    }
    let (warnings, ordinary): (Vec<_>, Vec<_>) =
        entries.into_iter().partition(|v| !v.reasons.is_empty());
    let mut ordered = Vec::new();
    // Applicable core constraints take precedence, then reserve capacity for review warnings.
    for entry in ordinary.iter().filter(|v| v.memory.tier == Tier::Core) {
        ordered.push(entry.clone());
    }
    let reserved = budget / 4;
    let mut warning_used = 0;
    let mut remaining_warnings = Vec::new();
    for entry in warnings {
        let size = counter.count(&serde_json::to_string(&entry)?);
        if warning_used == 0 || warning_used + size <= reserved {
            warning_used += size;
            ordered.push(entry);
        } else {
            remaining_warnings.push(entry);
        }
    }
    ordered.extend(ordinary.into_iter().filter(|v| v.memory.tier != Tier::Core));
    ordered.extend(remaining_warnings);
    for mut entry in ordered {
        let mandatory = entry.memory.tier == Tier::Core && entry.reasons.is_empty();
        let mut candidate = pack.clone();
        candidate
            .retrieved_memory_refs
            .push(entry.reference.clone());
        if entry.reasons.is_empty() {
            candidate.memories.push(entry.clone());
        } else {
            candidate.needs_verification.push(entry.clone());
        }
        if packed_cost(&candidate, counter)? > budget {
            entry.code_context.edges.clear();
            entry.code_context.paths.truncate(1);
            entry.code_context.nodes.truncate(3);
            entry.code_context.snippets.truncate(1);
            entry.code_context.truncated = true;
            if let Some(snippet) = entry.code_context.snippets.first_mut()
                && let Some(text) = snippet["text"].as_str()
            {
                snippet["text"] =
                    serde_json::json!(text.lines().take(8).collect::<Vec<_>>().join("\n"));
            }
            if entry.reasons.is_empty() {
                *candidate.memories.last_mut().expect("appended entry") = entry.clone();
            } else {
                *candidate
                    .needs_verification
                    .last_mut()
                    .expect("appended warning") = entry.clone();
            }
        }
        let tokens = packed_cost(&candidate, counter)?;
        if candidate.retrieved_memory_refs.len() <= limit && tokens <= budget {
            pack = candidate;
        } else {
            ensure!(
                !mandatory,
                "Applicable mandatory memory constraints exceed context budget"
            );
            pack.truncated += 1;
        }
    }
    if pack.retrieved_memory_refs.is_empty() {
        pack.status = if matches!(original_status.as_str(), "unavailable" | "disabled") {
            original_status
        } else {
            "empty".into()
        };
    }
    if partial {
        pack.status = "partial".into();
    }
    pack.estimated_tokens = packed_cost(&pack, counter)?;
    ensure!(
        pack.estimated_tokens <= budget,
        "Memory response wrapper exceeds context budget"
    );
    Ok(pack)
}
