//! Role tool dispatch: one generic tool per capability, served by the first
//! usable provider in the role's order with fallback past provider-side
//! failures.
use super::{SearchService, validate_arguments};
use crate::{Error, Result};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use tinysearch_bus::{
    ExecuteToolRequest, ExecuteToolResponse, Role, ToolSpec, role_for_tool, role_provider_tool,
    role_providers, role_tool_specs,
};

/// The provider that serves only `depth: "deep"` answers.
const DEEP_RESEARCH: &str = "gemini_deep_research";

impl SearchService {
    /// Executes a role tool across the role's providers.
    ///
    /// An explicit `provider` argument pins the call to that provider with no
    /// fallback. Otherwise providers are tried in role order, moving on only
    /// after a fallback-eligible error; `fallback_from` records the providers
    /// that failed before the one that answered.
    pub(super) async fn execute_role(
        &self,
        available: &BTreeMap<String, Vec<ToolSpec>>,
        request: ExecuteToolRequest,
    ) -> Result<ExecuteToolResponse> {
        let role = role_for_tool(&request.name)
            .ok_or_else(|| Error::UnavailableTool(request.name.clone()))?;
        let spec = role_tool_specs(available, &self.config.presentation, role)
            .ok_or_else(|| Error::UnavailableTool(request.name.clone()))?;
        validate_arguments(&spec, &request.arguments)?;
        let args = request
            .arguments
            .as_object()
            .ok_or(Error::InvalidArguments)?;
        let usable = role_providers(available, &self.config.presentation, role);
        let explicit = args.get("provider").and_then(Value::as_str);
        let candidates = match explicit {
            Some(provider) => vec![provider.to_owned()],
            None if role == Role::Answer => answer_order(usable, args),
            None => usable,
        };
        let mut failed: Vec<String> = Vec::new();
        let mut last_error = None;
        for provider in candidates {
            let tool = role_provider_tool(role, &provider)
                .and_then(|name| {
                    available
                        .get(&provider)?
                        .iter()
                        .find(|tool| tool.name == name)
                })
                .cloned()
                .ok_or_else(|| Error::UnavailableProvider(provider.clone()))?;
            let provider_request = ExecuteToolRequest {
                name: tool.name.clone(),
                arguments: provider_arguments(role, &provider, args, &tool),
            };
            match self.dispatch(&provider, &tool, provider_request).await {
                Ok(mut response) => {
                    response.role = Some(role);
                    response.fallback_from = failed;
                    return Ok(response);
                }
                Err(error) if explicit.is_none() && error.is_fallback_eligible() => {
                    failed.push(provider);
                    last_error = Some(error);
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or(Error::UnavailableTool(request.name)))
    }
}

/// Orders answer providers for the requested depth.
///
/// `deep` puts Deep Research first when it is usable and keeps the grounded
/// providers as fallbacks. `quick` never uses Deep Research. An absent depth
/// is `quick` unless Deep Research is the only usable provider.
fn answer_order(mut usable: Vec<String>, args: &Map<String, Value>) -> Vec<String> {
    let only_deep = usable.iter().all(|name| name == DEEP_RESEARCH);
    let deep = match args.get("depth").and_then(Value::as_str) {
        Some(depth) => depth == "deep",
        None => only_deep,
    };
    if deep {
        if let Some(index) = usable.iter().position(|name| name == DEEP_RESEARCH) {
            let research = usable.remove(index);
            usable.insert(0, research);
        }
    } else {
        usable.retain(|name| name != DEEP_RESEARCH);
    }
    usable
}

/// Translates generic role arguments into `provider`'s own tool arguments,
/// keeping only the fields the provider tool declares.
pub(super) fn provider_arguments(
    role: Role,
    provider: &str,
    args: &Map<String, Value>,
    tool: &ToolSpec,
) -> Value {
    let get = |key: &str| args.get(key).cloned().unwrap_or(Value::Null);
    let mut mapped = match role {
        Role::Search => {
            let count_field = if provider == "brave" {
                "count"
            } else {
                "max_results"
            };
            json!({"query":get("query"), count_field:get("max_results")})
        }
        Role::Answer => json!({"query":get("query")}),
        Role::Contents => {
            let mut mapped = json!({"urls":get("urls"),"query":get("query")});
            if provider == "tinyfish" {
                mapped["format"] = json!("markdown");
            }
            mapped
        }
    };
    let declared = tool
        .parameters
        .get("properties")
        .and_then(Value::as_object);
    if let Some(fields) = mapped.as_object_mut() {
        fields.retain(|key, value| {
            !value.is_null() && declared.is_some_and(|declared| declared.contains_key(key))
        });
    }
    mapped
}

#[cfg(test)]
mod test;
