use super::{Prepared, count, required_string, urls};
use crate::{Error, ExecuteToolRequest, Result};
use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;

pub(super) fn prepare(request: &ExecuteToolRequest, key: &str) -> Result<Prepared> {
    let args = &request.arguments;
    let (path, mut body, timeout) = match request.name.as_str() {
        "tavily_search" => (
            "/search",
            json!({"query":required_string(args,"query")?,"max_results":count(args,"max_results")}),
            Duration::from_secs(35),
        ),
        "tavily_extract" => (
            "/extract",
            json!({"urls":urls(args)?,"format":"markdown"}),
            Duration::from_secs(65),
        ),
        _ => return Err(Error::UnavailableTool(request.name.clone())),
    };
    let fields: &[&str] = if request.name == "tavily_search" {
        &[
            "search_depth",
            "topic",
            "time_range",
            "start_date",
            "end_date",
            "include_answer",
            "include_raw_content",
            "include_images",
            "include_domains",
            "exclude_domains",
        ]
    } else {
        &["format", "extract_depth"]
    };
    for field in fields {
        if let Some(v) = args.get(*field) {
            body[*field] = v.clone();
        }
    }
    Ok((
        Method::POST,
        "https://api.tavily.com",
        path.into(),
        Some(body),
        vec![],
        ("Authorization", format!("Bearer {key}")),
        timeout,
    ))
}

pub(super) fn unwrap(request: &ExecuteToolRequest, mut value: Value) -> Result<Value> {
    if request.name == "tavily_extract" {
        let failed = value
            .get("failed_results")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let empty = value
            .get("results")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty);
        if empty && failed > 0 {
            return Err(Error::Provider(format!(
                "Tavily extraction failed for {failed} URLs"
            )));
        }
        if let Some(results) = value.get_mut("results").and_then(Value::as_array_mut) {
            for item in results {
                if let Some(raw) = item.get("raw_content").cloned() {
                    item["snippet"] = raw;
                }
            }
        }
        value["failed_count"] = json!(failed);
    } else if request.arguments.get("include_answer") != Some(&Value::Bool(true)) {
        value.as_object_mut().map(|o| o.remove("answer"));
    }
    Ok(value)
}

#[cfg(test)]
#[path = "tavily/tavily_tests.rs"]
mod test;
