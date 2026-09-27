use super::{Prepared, count, required_string, urls};
use crate::{Error, ExecuteToolRequest, Result};
use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;

pub(super) fn prepare(request: &ExecuteToolRequest, key: &str) -> Result<Prepared> {
    let (path, body) = exa_body(request)?;
    Ok((
        Method::POST,
        "https://api.exa.ai",
        path.into(),
        Some(body),
        vec![],
        ("x-api-key", key.into()),
        Duration::from_secs(35),
    ))
}

/// Builds Exa's own request path and body for an Exa tool.
pub(super) fn exa_body(request: &ExecuteToolRequest) -> Result<(&'static str, Value)> {
    let args = &request.arguments;
    let (path, mut body) = match request.name.as_str() {
        "exa_search" => (
            "/search",
            json!({"query":required_string(args,"query")?,"numResults":count(args,"max_results")}),
        ),
        "exa_find_similar" => (
            "/findSimilar",
            json!({"url":required_string(args,"url")?,"numResults":count(args,"max_results")}),
        ),
        "exa_get_contents" => ("/contents", json!({"urls":urls(args)?,"text":true})),
        "exa_answer" => {
            let mut body = json!({"query":required_string(args,"query")?});
            if args.get("include_text") == Some(&Value::Bool(true)) {
                body["text"] = json!(true);
            }
            return Ok(("/answer", body));
        }
        _ => return Err(Error::UnavailableTool(request.name.clone())),
    };
    if request.name == "exa_search" {
        for (src, dst) in [
            ("type", "type"),
            ("category", "category"),
            ("start_published_date", "startPublishedDate"),
            ("end_published_date", "endPublishedDate"),
        ] {
            if let Some(v) = args.get(src) {
                body[dst] = v.clone();
            }
        }
    }
    if request.name == "exa_get_contents" {
        if let Some(query) = args.get("query").and_then(Value::as_str) {
            body["highlights"] = json!({"query":query});
        }
        for (src, dst) in [
            ("include_summary", "summary"),
            ("include_highlights", "highlights"),
        ] {
            if args.get(src) == Some(&Value::Bool(true)) && body.get(dst).is_none() {
                body[dst] = json!(true);
            }
        }
    } else {
        for (src, dst) in [
            ("include_domains", "includeDomains"),
            ("exclude_domains", "excludeDomains"),
        ] {
            if let Some(v) = args.get(src) {
                body[dst] = v.clone();
            }
        }
        if let Some(v) = args.get("exclude_source_domain") {
            body["excludeSourceDomain"] = v.clone();
        }
        let mut contents = json!({});
        if args.get("include_text") == Some(&Value::Bool(true)) {
            contents["text"] = json!(true);
        }
        if args.get("include_highlights") == Some(&Value::Bool(true)) {
            contents["highlights"] = json!(true);
        }
        if contents.as_object().is_some_and(|o| !o.is_empty()) {
            body["contents"] = contents;
        }
    }
    Ok((path, body))
}
