//! Bounded normalization of provider payloads.
use super::{MAX_ANSWER_CHARS, MAX_CITATIONS, MAX_GROUNDING_CHUNKS, MAX_RESULTS};
use crate::{Citation, ExecuteToolResponse, SearchResult, SearchStatus};
use serde_json::{Map, Value};

fn clipped(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}
fn get_text(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(|s| clipped(s, MAX_ANSWER_CHARS))
}
fn add_result(results: &mut Vec<SearchResult>, item: &Value) {
    if results.len() >= MAX_RESULTS {
        return;
    }
    let url = item.get("url").and_then(Value::as_str).unwrap_or("");
    if url.is_empty() {
        return;
    }
    let title = item.get("title").and_then(Value::as_str).unwrap_or("");
    let snippet = item
        .get("snippet")
        .and_then(Value::as_str)
        .or_else(|| item.get("summary").and_then(Value::as_str))
        .or_else(|| {
            item.get("highlights")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str)
        })
        .or_else(|| item.get("content").and_then(Value::as_str))
        .or_else(|| item.get("raw_content").and_then(Value::as_str))
        .or_else(|| {
            item.get("excerpts")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str)
        })
        .or_else(|| item.get("description").and_then(Value::as_str))
        .or_else(|| item.get("full_content").and_then(Value::as_str))
        .or_else(|| item.get("text").and_then(Value::as_str));
    results.push(SearchResult {
        title: clipped(title, 300),
        url: clipped(url, 2048),
        snippet: snippet.map(|s| clipped(s, 1200)),
        published: item
            .get("published_date")
            .or_else(|| item.get("publish_date"))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(|s| clipped(s, 100)),
    });
}
fn add_citation(citations: &mut Vec<Citation>, url: &str, title: Option<&str>) {
    if citations.len() < MAX_CITATIONS && !url.is_empty() && !citations.iter().any(|c| c.url == url)
    {
        citations.push(Citation {
            url: clipped(url, 2048),
            title: title.map(|s| clipped(s, 300)),
        });
    }
}
fn answer_for(tool: &str, value: &Value) -> Option<String> {
    match tool {
        "parallel_chat" => value
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(|s| clipped(s, MAX_ANSWER_CHARS)),
        "gemini_agentic_search" => gemini_text(value),
        "gemini_deep_research" => value
            .get("steps")
            .and_then(Value::as_array)
            .and_then(|a| a.last())
            .and_then(|v| v.get("content"))
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str)
            .or_else(|| value.get("output_text").and_then(Value::as_str))
            .map(|s| clipped(s, MAX_ANSWER_CHARS)),
        "parallel_research"
        | "parallel_enrich"
        | "parallel_dataset"
        | "parallel_research_status"
        | "parallel_enrich_status"
        | "parallel_dataset_status"
        | "tinyfish_agent_run" => value
            .get("result")
            .or_else(|| value.get("output"))
            .map(|v| clipped(v.as_str().unwrap_or(&v.to_string()), MAX_ANSWER_CHARS)),
        "tinyfish_fetch" => value
            .get("results")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|v| v.get("text"))
            .map(|v| clipped(v.as_str().unwrap_or(&v.to_string()), MAX_ANSWER_CHARS)),
        _ => get_text(value, "answer"),
    }
}

/// Joins the text parts of Gemini's first candidate, skipping thought parts.
///
/// Accumulates only up to `MAX_ANSWER_CHARS`: a response with many or very
/// large parts stops contributing characters once the limit is reached
/// instead of first concatenating the whole answer and clipping afterward.
fn gemini_text(value: &Value) -> Option<String> {
    let parts = value
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)?;
    let mut text = String::new();
    let mut collected = 0_usize;
    for part in parts {
        if collected >= MAX_ANSWER_CHARS {
            break;
        }
        if part.get("thought") == Some(&Value::Bool(true)) {
            continue;
        }
        if let Some(part_text) = part.get("text").and_then(Value::as_str) {
            let remaining = MAX_ANSWER_CHARS - collected;
            let taken = part_text.chars().take(remaining);
            collected += taken.clone().count();
            text.extend(taken);
        }
    }
    (!text.is_empty()).then_some(text)
}

/// Adds Gemini grounding sources as citations: chunks that ground the answer
/// (referenced by `groundingSupports`) first, in order of first reference,
/// then any remaining web chunks.
fn add_grounding_citations(citations: &mut Vec<Citation>, value: &Value) {
    let Some(metadata) = value.pointer("/candidates/0/groundingMetadata") else {
        return;
    };
    let Some(chunks) = metadata.get("groundingChunks").and_then(Value::as_array) else {
        return;
    };
    // Only MAX_CITATIONS can be emitted, so stop collecting once that many
    // distinct chunks are ordered. Beyond bounding the *output*, cap how much
    // of the provider-controlled *input* is ever examined: `seen` is sized to
    // (and indices are drawn from) at most `MAX_GROUNDING_CHUNKS` chunks, and
    // the referenced-index scan is capped at the same count, so an
    // oversized `groundingChunks`/`groundingSupports` payload cannot force
    // allocation or traversal proportional to its own size.
    let chunk_count = chunks.len().min(MAX_GROUNDING_CHUNKS);
    let mut seen = vec![false; chunk_count];
    let mut order: Vec<usize> = Vec::with_capacity(MAX_CITATIONS);
    let referenced = metadata
        .get("groundingSupports")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|support| {
            support
                .get("groundingChunkIndices")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(Value::as_u64)
        .filter_map(|index| usize::try_from(index).ok())
        .take(MAX_GROUNDING_CHUNKS);
    // A chunk without a usable web URI never becomes a citation (see below),
    // so it must not consume an ordering slot that a later, usable chunk
    // could otherwise fill. Nor should a chunk whose URL duplicates one
    // already selected: `add_citation` below would just drop it, so counting
    // it against MAX_CITATIONS here would let a duplicate crowd out a later,
    // distinct URL.
    let usable_url = |index: usize| -> Option<&str> {
        chunks[index]
            .get("web")
            .and_then(|web| web.get("uri"))
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
    };
    let mut selected_urls: std::collections::HashSet<&str> = std::collections::HashSet::new();
    // Then the chunks no support referenced, in document order.
    for index in referenced.chain(0..chunk_count) {
        if order.len() >= MAX_CITATIONS {
            break;
        }
        if index < chunk_count
            && !seen[index]
            && let Some(url) = usable_url(index)
        {
            seen[index] = true;
            if selected_urls.insert(url) {
                order.push(index);
            }
        }
    }
    for index in order {
        if citations.len() >= MAX_CITATIONS {
            break;
        }
        if let Some(web) = chunks[index].get("web")
            && let Some(url) = web.get("uri").and_then(Value::as_str)
        {
            add_citation(citations, url, web.get("title").and_then(Value::as_str));
        }
    }
}

pub(super) fn normalize(provider: &str, tool: &str, value: &Value) -> ExecuteToolResponse {
    let mut results = Vec::new();
    let mut citations = Vec::new();
    if let Some(items) = value.get("results").and_then(Value::as_array) {
        for item in items {
            add_result(&mut results, item);
            if results.len() == MAX_RESULTS {
                break;
            }
        }
    }
    for result in &results {
        add_citation(&mut citations, &result.url, Some(&result.title));
    }
    if let Some(items) = value
        .get("basis")
        .and_then(Value::as_array)
        .or_else(|| value.get("citations").and_then(Value::as_array))
    {
        for item in items.iter().take(MAX_CITATIONS) {
            if let Some(url) = item.get("url").and_then(Value::as_str) {
                add_citation(
                    &mut citations,
                    url,
                    item.get("title").and_then(Value::as_str),
                );
            }
        }
    }
    if let Some(steps) = value.get("steps").and_then(Value::as_array) {
        for step in steps.iter().rev().take(4) {
            if let Some(blocks) = step.get("content").and_then(Value::as_array) {
                for block in blocks.iter().take(8) {
                    if let Some(annotations) = block.get("annotations").and_then(Value::as_array) {
                        for annotation in annotations.iter().take(MAX_CITATIONS) {
                            if let Some(url) = annotation.get("url").and_then(Value::as_str) {
                                add_citation(
                                    &mut citations,
                                    url,
                                    annotation.get("title").and_then(Value::as_str),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    add_grounding_citations(&mut citations, value);
    let answer = answer_for(tool, value);
    let mut meta = Map::new();
    for field in [
        "id",
        "requestId",
        "runId",
        "searchId",
        "run_id",
        "findall_id",
        "search_id",
        "extract_id",
        "status",
        "costUsd",
        "total_results",
        "num_of_steps",
        "failed_count",
    ] {
        if let Some(v) = value.get(field)
            && (v.is_number() || v.is_boolean() || v.as_str().is_some_and(|s| s.len() <= 128))
        {
            meta.insert(field.into(), v.clone());
        }
    }
    let status = if results.is_empty() && answer.as_deref().is_none_or(str::is_empty) {
        SearchStatus::Empty
    } else {
        SearchStatus::Ok
    };
    ExecuteToolResponse {
        provider: provider.into(),
        results,
        citations,
        answer,
        status,
        provider_data: (!meta.is_empty()).then_some(Value::Object(meta)),
        role: None,
        fallback_from: Vec::new(),
    }
}
