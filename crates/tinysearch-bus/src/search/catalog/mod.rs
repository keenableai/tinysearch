//! Tool schemas and presentation selection shared by hosts and modules.
use super::{
    ListToolsResponse, PresentationConfig, PresentationMode, ProviderRoute, Role, SearchConfig,
    ToolSpec,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

mod roles;
pub use roles::{
    default_role_providers, provider_roles, role_provider_tool, role_providers, role_tool_specs,
};

/// Every provider the module implements, by stable name.
pub const PROVIDERS: &[&str] = &[
    "exa",
    "gemini",
    "gemini_deep_research",
    "tinyfish",
    "parallel",
    "brave",
    "querit",
    "tavily",
    "seltz",
    "searxng",
];

/// Providers that support [`ProviderRoute::Backend`] through the managed
/// backend. Only these become usable from a backend credential; every other
/// provider, including `parallel` and `tinyfish`, is direct-only (bring your
/// own key). The managed backend does not proxy `tinyfish`.
pub const BACKEND_PROVIDERS: &[&str] = &["exa", "gemini"];

/// Providers usable directly with their own private credential.
const KEYED_DIRECT_PROVIDERS: &[&str] = &[
    "exa",
    "brave",
    "querit",
    "tavily",
    "gemini",
    "gemini_deep_research",
    "seltz",
    "parallel",
    "tinyfish",
];

/// Gemini models the managed backend accepts for grounded generation.
const GEMINI_BACKEND_MODELS: &[&str] = &[
    "gemini-3.8-flash",
    "gemini-3.5-flash",
    "gemini-3.5-flash-lite",
    "gemini-3.1-flash-lite",
    "gemini-3.1-pro-preview",
];

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> ToolSpec {
    let mut parameters = json!({"type":"object","required":required,"additionalProperties":false});
    parameters["properties"] = properties;
    ToolSpec {
        name: name.into(),
        description: description.into(),
        parameters,
    }
}

/// Returns the supported provider operations and their exact input contracts.
#[must_use]
pub fn provider_tool_specs() -> BTreeMap<String, Vec<ToolSpec>> {
    let text = json!({"type":"string","minLength":1});
    let urls = |max| json!({"type":"array","items":{"type":"string","minLength":1},"minItems":1,"maxItems":max});
    let tinyfish = vec![
        tool(
            "tinyfish_search",
            "Search with TinyFish",
            json!({"query":text,"location":{"type":"string"},"language":{"type":"string"},"page":{"type":"integer","minimum":0,"maximum":10}}),
            &["query"],
        ),
        tool(
            "tinyfish_fetch",
            "Render pages with TinyFish",
            json!({"urls":urls(10),"format":{"type":"string","enum":["markdown","html","json"]}}),
            &["urls"],
        ),
        tool(
            "tinyfish_agent_run",
            "Run TinyFish browser automation",
            json!({"url":text,"goal":text,"output_schema":{"type":"object"},"browser_profile":{"type":"string","enum":["lite","stealth"]},"proxy_country_code":{"type":"string","enum":["US","GB","CA","DE","FR","JP","AU"]},"use_vault":{"type":"boolean"},"credential_item_ids":{"type":"array","items":{"type":"string"}}}),
            &["url", "goal"],
        ),
    ];
    let gemini = vec![tool(
        "gemini_agentic_search",
        "Search with Gemini grounded by Google Search",
        json!({"query":text,"model":{"type":"string","minLength":1}}),
        &["query"],
    )];
    let deep = vec![tool(
        "gemini_deep_research",
        "Research with Gemini Deep Research",
        json!({"query":text,"interaction_id":{"type":"string","minLength":1},"max_poll_attempts":{"type":"integer","minimum":1,"maximum":30}}),
        &[],
    )];
    let mut specs: BTreeMap<String, Vec<ToolSpec>> = [
        ("tinyfish".into(), tinyfish),
        ("gemini".into(), gemini),
        ("gemini_deep_research".into(), deep),
        (
            "searxng".into(),
            vec![tool(
                "searxng_search",
                "Search a SearXNG instance",
                json!({"query":text,"categories":{"type":"array","items":{"type":"string","enum":["web","general","news","images"]}},"language":text,"max_results":{"type":"integer","minimum":1,"maximum":50}}),
                &["query"],
            )],
        ),
    ]
    .into();
    specs.extend(direct_provider_specs());
    specs
}

fn direct_provider_specs() -> BTreeMap<String, Vec<ToolSpec>> {
    let text = json!({"type":"string","minLength":1});
    let urls = |max| json!({"type":"array","items":{"type":"string","minLength":1},"minItems":1,"maxItems":max});
    let exa = vec![
        tool(
            "exa_search",
            "Search with Exa",
            json!({"query":text,"max_results":{"type":"integer","minimum":1,"maximum":20},"type":{"type":"string","enum":["auto","instant","fast","deep-lite","deep","deep-reasoning"]},"category":text,"include_domains":urls(20),"exclude_domains":urls(20),"start_published_date":text,"end_published_date":text,"include_text":{"type":"boolean"},"include_highlights":{"type":"boolean"}}),
            &["query"],
        ),
        tool(
            "exa_find_similar",
            "Find pages similar to a URL with Exa",
            json!({"url":text,"max_results":{"type":"integer","minimum":1,"maximum":20},"exclude_source_domain":{"type":"boolean"},"include_domains":urls(20),"exclude_domains":urls(20),"include_text":{"type":"boolean"},"include_highlights":{"type":"boolean"}}),
            &["url"],
        ),
        tool(
            "exa_get_contents",
            "Get page contents with Exa",
            json!({"urls":urls(20),"query":text,"include_summary":{"type":"boolean"},"include_highlights":{"type":"boolean"}}),
            &["urls"],
        ),
        tool(
            "exa_answer",
            "Answer a question with Exa, grounded in cited web results",
            json!({"query":text,"include_text":{"type":"boolean"}}),
            &["query"],
        ),
    ];
    let brave = ["web", "news", "image", "video"].into_iter().map(|kind| {
        let mut properties = json!({"query":text,"count":{"type":"integer","minimum":1,"maximum":20},"country":{"type":"string","minLength":2,"maxLength":2}});
        if kind != "image" { properties["freshness"] = json!({"type":"string","minLength":1}); }
        tool(&format!("brave_{kind}_search"), &format!("Search {kind} with Brave"), properties, &["query"])
    }).collect();
    let querit = vec![tool(
        "querit_search",
        "Search with Querit",
        json!({"query":text,"max_results":{"type":"integer","minimum":1,"maximum":20},"filters":{"type":"object"},"include_domains":urls(20),"exclude_domains":urls(20),"time_range":text,"date":text,"from_date":text,"to_date":text,"countries":urls(20),"languages":urls(20)}),
        &["query"],
    )];
    let tavily = vec![
        tool(
            "tavily_search",
            "Search with Tavily",
            json!({"query":text,"max_results":{"type":"integer","minimum":1,"maximum":20},"search_depth":{"type":"string","enum":["basic","advanced","fast","ultra-fast"]},"topic":{"type":"string","enum":["general","news","finance"]},"time_range":{"type":"string","enum":["day","week","month","year"]},"start_date":text,"end_date":text,"include_answer":{"type":"boolean"},"include_raw_content":{"type":"boolean"},"include_images":{"type":"boolean"},"include_domains":urls(20),"exclude_domains":urls(20)}),
            &["query"],
        ),
        tool(
            "tavily_extract",
            "Extract pages with Tavily",
            json!({"urls":urls(20),"format":{"type":"string","enum":["markdown","text"]},"extract_depth":{"type":"string","enum":["basic","advanced"]}}),
            &["urls"],
        ),
    ];
    let seltz = vec![tool(
        "seltz_search",
        "Search the web with Seltz",
        json!({"query":text,"max_results":{"type":"integer","minimum":1,"maximum":20},"include_domains":urls(20),"exclude_domains":urls(20),"from_date":text,"to_date":text,"scope":{"type":"string","enum":["news"]}}),
        &["query"],
    )];
    [
        ("exa".into(), exa),
        ("parallel".into(), parallel_specs()),
        ("brave".into(), brave),
        ("querit".into(), querit),
        ("tavily".into(), tavily),
        ("seltz".into(), seltz),
    ]
    .into()
}

/// Parallel's direct API operations. Parallel has no managed backend route, so
/// these are the direct schemas: current search modes, extract without the
/// backend-only `excerpts` switch, async runs without a server-side wait, and
/// status tools that resume a Task or `FindAll` run by ID.
fn parallel_specs() -> Vec<ToolSpec> {
    let text = json!({"type":"string","minLength":1});
    let urls = |max| json!({"type":"array","items":{"type":"string","minLength":1},"minItems":1,"maxItems":max});
    let input = json!({"oneOf":[{"type":"string","minLength":1},{"type":"object"}]});
    let processor = json!({"type":"string","enum":["lite","base","core","ultra"]});
    let id = json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_-]+$"});
    vec![
        tool(
            "parallel_search",
            "Search the web with Parallel",
            json!({"objective":text,"search_queries":urls(10),"mode":{"type":"string","enum":["turbo","fast","basic","advanced"]},"num_results":{"type":"integer","minimum":1,"maximum":50},"max_characters_per_excerpt":{"type":"integer","minimum":100,"maximum":10000}}),
            &["objective", "search_queries"],
        ),
        tool(
            "parallel_extract",
            "Extract web pages with Parallel",
            json!({"urls":urls(20),"objective":text,"full_content":{"type":"boolean"}}),
            &["urls"],
        ),
        tool(
            "parallel_chat",
            "Ask Parallel's web grounded chat",
            json!({"model":{"type":"string","enum":["speed","lite","base","core"]},"messages":{"type":"array","minItems":1,"items":{"type":"object","properties":{"role":{"type":"string","enum":["system","user","assistant"]},"content":text},"required":["role","content"],"additionalProperties":false}}}),
            &["model", "messages"],
        ),
        tool(
            "parallel_research",
            "Run Parallel research",
            json!({"input":input,"processor":processor,"output_schema":{"type":"object"}}),
            &["input", "processor"],
        ),
        tool(
            "parallel_enrich",
            "Enrich an entity with Parallel",
            json!({"input":input,"processor":processor,"output_schema":{"type":"object"}}),
            &["input", "processor", "output_schema"],
        ),
        tool(
            "parallel_dataset",
            "Build a dataset with Parallel",
            json!({"objective":text,"entity_type":text,"match_conditions":{"type":"array","minItems":1,"maxItems":20,"items":{"type":"object","properties":{"name":text,"description":text},"required":["name","description"],"additionalProperties":false}},"generator":{"type":"string","enum":["preview","base","core","pro"]},"match_limit":{"type":"integer","minimum":5,"maximum":1000}}),
            &["objective", "entity_type", "match_conditions"],
        ),
        tool(
            "parallel_research_status",
            "Check a Parallel research run and fetch its result",
            json!({"run_id":id}),
            &["run_id"],
        ),
        tool(
            "parallel_enrich_status",
            "Check a Parallel enrichment run and fetch its result",
            json!({"run_id":id}),
            &["run_id"],
        ),
        tool(
            "parallel_dataset_status",
            "Check a Parallel dataset run and fetch its result",
            json!({"findall_id":id}),
            &["findall_id"],
        ),
    ]
}

/// Filters declared providers by configuration and available credentials.
///
/// A provider is available only when the host configured it explicitly and
/// enabled it, and its chosen route is usable: [`ProviderRoute::Backend`]
/// needs a backend credential and a provider in [`BACKEND_PROVIDERS`];
/// [`ProviderRoute::Direct`] needs the provider's own non-empty credential
/// (or, for `searxng`, a non-empty base URL). Parallel is direct-only: a
/// backend route leaves it unavailable. Tool schemas are narrowed to
/// what the chosen route accepts.
#[must_use]
pub fn configured_provider_tools(
    config: &SearchConfig,
    specs: &BTreeMap<String, Vec<ToolSpec>>,
) -> BTreeMap<String, Vec<ToolSpec>> {
    if !config.enabled {
        return BTreeMap::new();
    }
    specs
        .iter()
        .filter_map(|(name, tools)| {
            let explicit = config.providers.get(name)?;
            let non_empty = |value: Option<&str>| value.is_some_and(|v| !v.trim().is_empty());
            let usable = match explicit.route {
                ProviderRoute::Backend => {
                    non_empty(config.backend.credential.as_deref())
                        && BACKEND_PROVIDERS.contains(&name.as_str())
                }
                ProviderRoute::Direct if name == "searxng" => {
                    non_empty(explicit.base_url.as_deref())
                }
                ProviderRoute::Direct => {
                    KEYED_DIRECT_PROVIDERS.contains(&name.as_str())
                        && non_empty(explicit.credential.as_deref())
                }
            };
            (explicit.enabled && usable)
                .then(|| (name.clone(), narrow_for_route(tools, explicit.route)))
        })
        .collect()
}

/// Narrows provider schemas to the arguments the chosen route forwards.
fn narrow_for_route(tools: &[ToolSpec], route: ProviderRoute) -> Vec<ToolSpec> {
    let mut tools = tools.to_vec();
    if route != ProviderRoute::Backend {
        return tools;
    }
    for tool in &mut tools {
        match tool.name.as_str() {
            // The backend's Exa search takes only an objective and queries.
            "exa_search" => {
                if let Some(properties) = tool
                    .parameters
                    .get_mut("properties")
                    .and_then(Value::as_object_mut)
                {
                    properties.retain(|key, _| key == "query");
                }
            }
            "gemini_agentic_search" => {
                tool.parameters["properties"]["model"] =
                    json!({"type":"string","enum":GEMINI_BACKEND_MODELS});
            }
            _ => {}
        }
    }
    tools
}

/// Selects deterministic presentation from available provider declarations.
#[must_use]
pub fn select_tools(
    provider_tools: &BTreeMap<String, Vec<ToolSpec>>,
    presentation: &PresentationConfig,
) -> ListToolsResponse {
    match presentation.mode {
        PresentationMode::Roles => ListToolsResponse {
            tools: Role::ALL
                .into_iter()
                .filter_map(|role| role_tool_specs(provider_tools, presentation, role))
                .collect(),
        },
        PresentationMode::AllTools => ListToolsResponse {
            tools: provider_tools.values().flatten().cloned().collect(),
        },
        PresentationMode::OneProvider => ListToolsResponse {
            tools: presentation
                .provider
                .as_ref()
                .and_then(|name| provider_tools.get(name))
                .cloned()
                .unwrap_or_default(),
        },
        PresentationMode::Router => {
            if provider_tools.is_empty()
                || role_providers(provider_tools, presentation, Role::Search).is_empty()
            {
                return ListToolsResponse::default();
            }
            // Router forwards provider-specific options, so it must advertise that fact.
            ListToolsResponse {
                tools: vec![ToolSpec {
                    name: "search".into(),
                    description: "Search with an available provider".into(),
                    parameters: json!({"type":"object","properties":{"provider":{"type":"string"},"query":{"type":"string","minLength":1}},"required":["query"],"additionalProperties":true}),
                }],
            }
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod test;
