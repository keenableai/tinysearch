//! Direct provider request mappings and bounded HTTP transport.
use super::{direct_url, normalize, required_string};
use crate::{
    Error, ExecuteToolRequest, ExecuteToolResponse, ProviderConfig, ProviderRoute, Result,
};
use reqwest::{Client, Method};
use serde_json::Value;
use std::time::Duration;

mod brave;
mod exa;
#[cfg(test)]
mod migration_tests;
mod querit;
mod searxng;
mod seltz;
mod tavily;

pub(super) async fn run(
    client: &Client,
    provider: &str,
    config: &ProviderConfig,
    request: &ExecuteToolRequest,
) -> Result<ExecuteToolResponse> {
    if config.route != ProviderRoute::Direct {
        return Err(Error::Provider(
            "this provider requires a direct route".into(),
        ));
    }
    if provider == "searxng" {
        return searxng::run(client, config, request).await;
    }
    let key = config
        .credential
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| Error::Provider("provider credential unavailable".into()))?;
    let (method, default_base, path, body, params, header, timeout) =
        prepare(provider, config, request, key)?;
    let value = send(
        client,
        config,
        (method, default_base, path, body, params, header, timeout),
    )
    .await?;
    normalize_response(provider, config, request, value)
}

async fn send(client: &Client, config: &ProviderConfig, prepared: Prepared) -> Result<Value> {
    let (method, default_base, path, body, params, header, timeout) = prepared;
    let url = direct_url(config.base_url.as_deref(), default_base, &path)?;
    let mut builder = client
        .request(method, url)
        .timeout(configured_timeout(config, timeout))
        .header(reqwest::header::ACCEPT, "application/json")
        .header(header.0, header.1);
    if let Some(body) = body {
        builder = builder.json(&body);
    }
    if !params.is_empty() {
        builder = builder.query(&params);
    }
    let response = builder
        .send()
        .await
        .map_err(super::http::transport_error)?;
    super::http::read_json(response).await
}

/// The Exa API path and body for an Exa tool, shared by the direct and
/// managed backend routes.
pub(super) fn exa_body(request: &ExecuteToolRequest) -> Result<(&'static str, Value)> {
    exa::body(request)
}

fn normalize_response(
    provider: &str,
    config: &ProviderConfig,
    request: &ExecuteToolRequest,
    value: Value,
) -> Result<ExecuteToolResponse> {
    let value = match provider {
        "brave" => brave::unwrap(request, &value),
        "querit" => querit::unwrap(&value)?,
        "tavily" => tavily::unwrap(request, value)?,
        "seltz" => seltz::unwrap(
            &value,
            request
                .arguments
                .get("max_results")
                .and_then(Value::as_u64)
                .or(config.max_results)
                .unwrap_or(5)
                .clamp(1, 20)
                .try_into()
                .unwrap_or(20),
        ),
        _ => value,
    };
    let mut result = normalize(provider, &request.name, &value);
    if request.name == "brave_image_search" {
        let assets: Vec<Value> = value
            .get("results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(super::MAX_RESULTS)
            .filter_map(|item| {
                let source_url = item.get("url")?.as_str()?;
                let image_url = item.pointer("/properties/url")?.as_str()?;
                Some(serde_json::json!({"source_url":source_url,"image_url":image_url}))
            })
            .collect();
        if !assets.is_empty() {
            result
                .provider_data
                .get_or_insert_with(|| serde_json::json!({}))["image_assets"] =
                Value::Array(assets);
        }
    }
    Ok(result)
}

pub(super) fn prepare(
    provider: &str,
    config: &ProviderConfig,
    request: &ExecuteToolRequest,
    key: &str,
) -> Result<Prepared> {
    let mut request = request.clone();
    let count_field = if provider == "brave" {
        "count"
    } else {
        "max_results"
    };
    if request.arguments.get(count_field).is_none()
        && !(provider == "querit" && request.arguments.get("count").is_some())
        && let Some(default_count) = config.max_results
        && let Some(args) = request.arguments.as_object_mut()
    {
        args.insert(count_field.into(), Value::from(default_count.clamp(1, 20)));
    }
    match provider {
        "exa" => exa::prepare(&request, key),
        "brave" => brave::prepare(&request, key),
        "querit" => querit::prepare(&request, key),
        "tavily" => tavily::prepare(&request, key),
        "seltz" => seltz::prepare(&request, key),
        _ => Err(Error::UnavailableProvider(provider.into())),
    }
}

pub(super) fn configured_timeout(config: &ProviderConfig, default: Duration) -> Duration {
    config
        .timeout_secs
        .map_or(default, |seconds| Duration::from_secs(seconds.max(1)))
}

type Prepared = (
    Method,
    &'static str,
    String,
    Option<Value>,
    Vec<(&'static str, String)>,
    (&'static str, String),
    Duration,
);
fn count(args: &Value, field: &str) -> u64 {
    args.get(field)
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .clamp(1, 20)
}
fn urls(args: &Value) -> Result<Value> {
    let items = args
        .get("urls")
        .and_then(Value::as_array)
        .ok_or(Error::InvalidArguments)?;
    if items.is_empty()
        || items.len() > 20
        || items
            .iter()
            .any(|v| v.as_str().is_none_or(|s| s.trim().is_empty()))
    {
        return Err(Error::InvalidArguments);
    }
    Ok(Value::Array(items.clone()))
}
