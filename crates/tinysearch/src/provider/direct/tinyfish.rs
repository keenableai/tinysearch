//! The `tinyfish` provider, reached directly with the user's own key.
//!
//! The provider splits its API across three hosts, all authenticated with an
//! `X-API-Key` header (<https://docs.tinyfish.ai>):
//! - Search: `GET https://api.search.tinyfish.ai?query=…` →
//!   `{query, results: [{position, site_name, title, snippet, url}], …}`
//! - Fetch: `POST https://api.fetch.tinyfish.ai` `{urls, format}` →
//!   `{results: [{url, final_url, title, text, …}], errors: [{url, error}]}`
//! - Agent: `POST https://agent.tinyfish.ai/v1/automation/run`
//!   `{url, goal, …}` → `{run_id, status, result, …}`
//!
//! The responses are already in the shapes `normalize` reads for `tinyfish`, so
//! no unwrapping is needed. A `base_url` override replaces the host for every
//! tool (a test server or a proxy).
use super::{Prepared, urls};
use crate::provider::{mapped, required_string};
use crate::{Error, ExecuteToolRequest, Result};
use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;

pub(super) const SEARCH_BASE: &str = "https://api.search.tinyfish.ai";
pub(super) const FETCH_BASE: &str = "https://api.fetch.tinyfish.ai";
pub(super) const AGENT_BASE: &str = "https://agent.tinyfish.ai";

/// Most URLs one fetch call accepts (the provider's own limit).
const MAX_FETCH_URLS: usize = 10;

pub(super) fn prepare(request: &ExecuteToolRequest, key: &str) -> Result<Prepared> {
    let auth = ("X-API-Key", key.to_owned());
    let args = &request.arguments;
    match request.name.as_str() {
        "tinyfish_search" => {
            let mut params = vec![("query", required_string(args, "query")?.to_owned())];
            for field in ["location", "language"] {
                if let Some(value) = args.get(field).and_then(Value::as_str) {
                    params.push((field, value.to_owned()));
                }
            }
            if let Some(page) = args.get("page").and_then(Value::as_u64) {
                params.push(("page", page.min(10).to_string()));
            }
            Ok((
                Method::GET,
                SEARCH_BASE,
                String::new(),
                None,
                params,
                auth,
                Duration::from_secs(35),
            ))
        }
        "tinyfish_fetch" => {
            let urls = urls(args)?;
            if urls
                .as_array()
                .is_some_and(|list| list.len() > MAX_FETCH_URLS)
            {
                return Err(Error::InvalidArguments);
            }
            let mut body = json!({ "urls": urls });
            if let Some(format) = args.get("format").and_then(Value::as_str) {
                body["format"] = Value::from(format);
            }
            Ok((
                Method::POST,
                FETCH_BASE,
                String::new(),
                Some(body),
                Vec::new(),
                auth,
                Duration::from_secs(60),
            ))
        }
        "tinyfish_agent_run" => {
            required_string(args, "url")?;
            required_string(args, "goal")?;
            let mut body = mapped(
                args,
                &[
                    ("url", "url"),
                    ("goal", "goal"),
                    ("output_schema", "output_schema"),
                    ("browser_profile", "browser_profile"),
                    ("use_vault", "use_vault"),
                    ("credential_item_ids", "credential_item_ids"),
                ],
            );
            if let Some(country) = args.get("proxy_country_code") {
                body["proxy_config"] =
                    json!({"enabled":true,"type":"tetra","country_code":country});
            }
            Ok((
                Method::POST,
                AGENT_BASE,
                "/v1/automation/run".into(),
                Some(body),
                Vec::new(),
                auth,
                // A synchronous browser run can take minutes.
                Duration::from_secs(300),
            ))
        }
        _ => Err(Error::UnavailableTool(request.name.clone())),
    }
}

#[cfg(test)]
#[path = "tinyfish/tinyfish_tests.rs"]
mod test;
