//! Keenable search and page fetch, usable without a credential.
//!
//! Without a credential, requests go to Keenable's public endpoints, which are
//! rate limited per IP and identify the caller by the `X-Keenable-Title`
//! header. A configured credential switches both tools to the keyed endpoints,
//! which have higher limits, and is sent as `X-API-Key`.
use super::{configured_timeout, direct_url, normalize, required_string, urls};
use crate::{Error, ExecuteToolRequest, ExecuteToolResponse, ProviderConfig, Result};
use reqwest::{Client, RequestBuilder, StatusCode};
use serde_json::{Value, json};
use std::time::Duration;

const BASE: &str = "https://api.keenable.ai/v1";
/// Names the calling software. Keenable rejects a keyless request without it;
/// it carries no user, host, or installation identifier.
const APP_TITLE: &str = "tinysearch";
/// Characters of page text requested per search result. Normalization keeps at
/// most this many per snippet, so a longer excerpt would only be discarded.
const SNIPPET_CHARS: u64 = 1200;

pub(super) async fn run(
    client: &Client,
    config: &ProviderConfig,
    request: &ExecuteToolRequest,
) -> Result<ExecuteToolResponse> {
    let key = config
        .credential
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    let value = match request.name.as_str() {
        "keenable_search" => search(client, config, key, &request.arguments).await?,
        "keenable_fetch" => fetch(client, config, key, &request.arguments).await?,
        _ => return Err(Error::UnavailableTool(request.name.clone())),
    };
    Ok(normalize("keenable", &request.name, &value))
}

/// The keyed endpoint, or its keyless `/public` twin when there is no key.
fn endpoint(config: &ProviderConfig, key: Option<&str>, path: &str) -> Result<String> {
    let suffix = if key.is_some() { "" } else { "/public" };
    direct_url(config.base_url.as_deref(), BASE, &format!("{path}{suffix}"))
}

fn identified(builder: RequestBuilder, key: Option<&str>) -> RequestBuilder {
    let builder = builder
        .header(reqwest::header::ACCEPT, "application/json")
        .header("X-Keenable-Title", APP_TITLE);
    match key {
        Some(key) => builder.header("X-API-Key", key),
        None => builder,
    }
}

async fn search(
    client: &Client,
    config: &ProviderConfig,
    key: Option<&str>,
    args: &Value,
) -> Result<Value> {
    let mut body = json!({
        "query": required_string(args, "query")?,
        "max_results": args
            .get("max_results")
            .and_then(Value::as_u64)
            .or(config.max_results)
            .unwrap_or(5)
            .clamp(1, 20),
        "snippet_max_length": SNIPPET_CHARS,
    });
    for field in ["site", "published_after", "published_before"] {
        if let Some(value) = args.get(field) {
            body[field] = value.clone();
        }
    }
    let response = identified(client.post(endpoint(config, key, "/search")?), key)
        .json(&body)
        .timeout(configured_timeout(config, Duration::from_secs(15)))
        .send()
        .await
        .map_err(super::super::http::transport_error)?;
    let value = super::super::http::read_json(response).await?;
    Ok(search_results(&value))
}

/// Maps Keenable results onto the fields normalization reads. The page text
/// is in `snippet`; `description` is usually empty and only a fallback.
fn search_results(value: &Value) -> Value {
    let results: Vec<Value> = value
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| {
            let text = |field: &str| {
                item.get(field)
                    .and_then(Value::as_str)
                    .filter(|text| !text.trim().is_empty())
            };
            json!({
                "url": text("url"),
                "title": text("title"),
                "snippet": text("snippet").or_else(|| text("description")),
                "published_date": text("published_at"),
            })
        })
        .collect();
    json!({"results": results})
}

async fn fetch(
    client: &Client,
    config: &ProviderConfig,
    key: Option<&str>,
    args: &Value,
) -> Result<Value> {
    let urls = urls(args)?;
    let mut results = Vec::new();
    let mut first_error = None;
    for url in urls
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        match fetch_page(client, config, key, url).await {
            Ok(page) => results.push(json!({
                "url": page.get("url").and_then(Value::as_str).unwrap_or(url),
                "title": page.get("title").and_then(Value::as_str).unwrap_or(""),
                "snippet": page.get("content").and_then(Value::as_str).unwrap_or(""),
            })),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    // A partial failure still returns the pages that were read; a total one
    // keeps the first error's classification so the role can fall back.
    if results.is_empty()
        && let Some(error) = first_error
    {
        return Err(error);
    }
    let failed = urls
        .as_array()
        .map_or(0, Vec::len)
        .saturating_sub(results.len());
    Ok(json!({"results": results, "failed_count": failed}))
}

/// Reads Keenable's indexed copy of a page and, when the page is not indexed
/// (404), fetches it live from the source instead.
async fn fetch_page(
    client: &Client,
    config: &ProviderConfig,
    key: Option<&str>,
    url: &str,
) -> Result<Value> {
    let send = |live: bool| {
        let mut builder =
            identified(client.get(endpoint(config, key, "/fetch")?), key).query(&[("url", url)]);
        if live {
            builder = builder.query(&[("live", "true")]);
        }
        Ok::<_, Error>(
            builder
                .timeout(configured_timeout(config, Duration::from_secs(30)))
                .send(),
        )
    };
    let response = send(false)?
        .await
        .map_err(super::super::http::transport_error)?;
    let response = if response.status() == StatusCode::NOT_FOUND {
        send(true)?
            .await
            .map_err(super::super::http::transport_error)?
    } else {
        response
    };
    super::super::http::read_json(response).await
}

#[cfg(test)]
#[path = "keenable/keenable_tests.rs"]
mod test;
