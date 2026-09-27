//! Direct Parallel API mapping. Task and `FindAll` runs are resumed by ID.
//!
//! Parallel is bring-your-own-key only: there is no managed backend route, so
//! every call goes to Parallel's own API with the provider credential.
use super::{configured_timeout, normalize};
use crate::{Error, ExecuteToolRequest, ExecuteToolResponse, ProviderConfig, Result, SearchStatus};
use reqwest::{Client, Method};
use serde_json::{Value, json};
use std::time::Duration;

const BASE: &str = "https://api.parallel.ai";

pub(super) async fn run(
    client: &Client,
    config: &ProviderConfig,
    request: &ExecuteToolRequest,
) -> Result<ExecuteToolResponse> {
    let key = config
        .credential
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| Error::Provider("provider credential unavailable".into()))?;
    let (method, path, body, kind) = prepare(config, request)?;
    let value = send(client, config, key, method, &path, body, kind).await?;
    if matches!(kind, Kind::Task | Kind::FindAll) {
        return async_response(client, config, key, request, kind, value).await;
    }
    Ok(normalize("parallel", &request.name, &value))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Sync,
    Task,
    FindAll,
}

fn prepare(
    config: &ProviderConfig,
    request: &ExecuteToolRequest,
) -> Result<(Method, String, Option<Value>, Kind)> {
    let args = &request.arguments;
    let get = |key: &str| args.get(key).cloned().ok_or(Error::InvalidArguments);
    let route = match request.name.as_str() {
        "parallel_search" => search_route(config, args)?,
        "parallel_extract" => extract_route(args)?,
        "parallel_chat" => (
            Method::POST,
            "/v1beta/chat/completions".into(),
            Some(json!({"model":get("model")?,"messages":get("messages")?,"stream":false})),
            Kind::Sync,
        ),
        "parallel_research" | "parallel_enrich" => {
            let mut body = json!({"input":get("input")?,"processor":get("processor")?});
            if let Some(schema) = args.get("output_schema") {
                body["task_spec"] = json!({"output_schema":{"type":"json","json_schema":schema}});
            }
            (
                Method::POST,
                "/v1/tasks/runs".into(),
                Some(body),
                Kind::Task,
            )
        }
        "parallel_dataset" => dataset_route(args)?,
        "parallel_research_status" | "parallel_enrich_status" => (
            Method::GET,
            format!("/v1/tasks/runs/{}", safe_id(&get("run_id")?)?),
            None,
            Kind::Task,
        ),
        "parallel_dataset_status" => (
            Method::GET,
            format!("/v1beta/findall/runs/{}", safe_id(&get("findall_id")?)?),
            None,
            Kind::FindAll,
        ),
        _ => {
            return Err(Error::Provider(
                "unsupported direct Parallel operation".into(),
            ));
        }
    };
    Ok(route)
}

fn search_route(
    config: &ProviderConfig,
    args: &Value,
) -> Result<(Method, String, Option<Value>, Kind)> {
    let get = |key: &str| args.get(key).cloned().ok_or(Error::InvalidArguments);
    let mut body = json!({"objective":get("objective")?,"search_queries":get("search_queries")?});
    if let Some(mode) = args.get("mode") {
        body["mode"] = mode.clone();
    }
    let mut settings = json!({});
    if let Some(count) = args.get("num_results") {
        settings["max_results"] = count.clone();
    } else if let Some(count) = config.max_results {
        settings["max_results"] = json!(count.clamp(1, 50));
    }
    if let Some(chars) = args.get("max_characters_per_excerpt") {
        settings["excerpt_settings"] = json!({"max_chars_per_result":chars});
    }
    if settings.as_object().is_some_and(|map| !map.is_empty()) {
        body["advanced_settings"] = settings;
    }
    Ok((Method::POST, "/v1/search".into(), Some(body), Kind::Sync))
}

fn extract_route(args: &Value) -> Result<(Method, String, Option<Value>, Kind)> {
    let urls = args.get("urls").cloned().ok_or(Error::InvalidArguments)?;
    let mut body = json!({"urls":urls});
    if let Some(objective) = args.get("objective") {
        body["objective"] = objective.clone();
    }
    if args
        .get("excerpts")
        .is_some_and(|excerpts| excerpts == false)
    {
        return Err(Error::Provider(
            "direct Parallel extract cannot disable excerpts".into(),
        ));
    }
    if let Some(full) = args.get("full_content") {
        body["advanced_settings"] = json!({"full_content":full});
    }
    Ok((Method::POST, "/v1/extract".into(), Some(body), Kind::Sync))
}

fn dataset_route(args: &Value) -> Result<(Method, String, Option<Value>, Kind)> {
    let get = |key: &str| args.get(key).cloned().ok_or(Error::InvalidArguments);
    let body = json!({"objective":get("objective")?,"entity_type":get("entity_type")?,"match_conditions":get("match_conditions")?,"generator":args.get("generator").cloned().unwrap_or(json!("base")),"match_limit":args.get("match_limit").cloned().unwrap_or(json!(100))});
    if body["match_conditions"]
        .as_array()
        .is_none_or(|conditions| {
            conditions.is_empty()
                || conditions.iter().any(|condition| {
                    ["name", "description"].iter().any(|field| {
                        condition
                            .get(field)
                            .and_then(Value::as_str)
                            .is_none_or(str::is_empty)
                    })
                })
        })
    {
        return Err(Error::InvalidArguments);
    }
    if body["match_limit"]
        .as_u64()
        .is_none_or(|n| !(5..=1000).contains(&n))
    {
        return Err(Error::InvalidArguments);
    }
    Ok((
        Method::POST,
        "/v1beta/findall/runs".into(),
        Some(body),
        Kind::FindAll,
    ))
}

fn safe_id(value: &Value) -> Result<&str> {
    let id = value.as_str().ok_or(Error::InvalidArguments)?;
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(Error::InvalidArguments);
    }
    Ok(id)
}

async fn send(
    client: &Client,
    config: &ProviderConfig,
    key: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
    kind: Kind,
) -> Result<Value> {
    let url = super::direct_url(config.base_url.as_deref(), BASE, path)?;
    let mut builder = client
        .request(method, url)
        .timeout(request_timeout(config, kind))
        .header(reqwest::header::ACCEPT, "application/json")
        .header("x-api-key", key);
    if let Some(body) = body {
        builder = builder.json(&body);
    }
    let response = builder
        .send()
        .await
        .map_err(super::super::http::transport_error)?;
    super::super::http::read_json(response).await
}

fn request_timeout(config: &ProviderConfig, kind: Kind) -> Duration {
    let timeout = configured_timeout(config, Duration::from_secs(35));
    if kind == Kind::Sync {
        timeout
    } else {
        timeout.min(Duration::from_secs(35))
    }
}

fn basis_citations(basis: &Value) -> Vec<Value> {
    basis
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|field| {
            field
                .get("citations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .take(super::super::MAX_CITATIONS)
        .cloned()
        .collect()
}

async fn async_response(
    client: &Client,
    config: &ProviderConfig,
    key: &str,
    request: &ExecuteToolRequest,
    kind: Kind,
    value: Value,
) -> Result<ExecuteToolResponse> {
    let (id_field, id) = if kind == Kind::Task {
        ("run_id", value.get("run_id"))
    } else {
        ("findall_id", value.get("findall_id"))
    };
    let id = id
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Provider("provider response missing run ID".into()))?;
    let id = safe_id(&json!(id))?.to_owned();
    let status = if kind == Kind::Task {
        value.get("status")
    } else {
        value.pointer("/status/status")
    };
    let status = status
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Provider("provider response missing status".into()))?;
    if matches!(status, "failed" | "cancelled" | "error") {
        return Err(Error::Provider("provider task failed".into()));
    }
    let mut output = if matches!(status, "completed" | "complete" | "succeeded") {
        let path = if kind == Kind::Task {
            format!("/v1/tasks/runs/{id}/result")
        } else {
            format!("/v1beta/findall/runs/{id}/result")
        };
        let result = send(client, config, key, Method::GET, &path, None, kind).await?;
        let mut response = if kind == Kind::FindAll {
            let candidates = result
                .get("candidates")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let matched: Vec<&Value> = candidates
                .iter()
                .filter(|candidate| {
                    candidate.get("match_status").and_then(Value::as_str) == Some("matched")
                        && candidate
                            .get("url")
                            .and_then(Value::as_str)
                            .is_some_and(|url| !url.is_empty())
                })
                .take(super::super::MAX_RESULTS)
                .collect();
            let results: Vec<Value> = matched.iter().filter_map(|candidate| {
                let url = candidate.get("url")?.as_str()?;
                Some(json!({"url":url,"title":candidate.get("name").and_then(Value::as_str).unwrap_or(""),"snippet":candidate.get("output").map(Value::to_string).unwrap_or_default()}))
            }).collect();
            let citations: Vec<Value> = matched
                .iter()
                .flat_map(|candidate| basis_citations(&candidate["basis"]))
                .take(super::super::MAX_CITATIONS)
                .collect();
            normalize(
                "parallel",
                &request.name,
                &json!({"results":results,"citations":citations}),
            )
        } else {
            let content = result
                .pointer("/output/content")
                .cloned()
                .unwrap_or(Value::Null);
            let basis = result
                .pointer("/output/basis")
                .cloned()
                .unwrap_or(Value::Null);
            let citations = basis_citations(&basis);
            normalize(
                "parallel",
                &request.name,
                &json!({"output":content,"citations":citations}),
            )
        };
        response.status = SearchStatus::Ok;
        response
    } else {
        let mut response = normalize("parallel", &request.name, &Value::Null);
        response.status = SearchStatus::InProgress;
        response
    };
    output.provider_data = Some(json!({id_field:id,"status":status}));
    Ok(output)
}

#[cfg(test)]
mod test;
