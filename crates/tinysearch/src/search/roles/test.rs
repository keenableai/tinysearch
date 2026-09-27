//! Role dispatch, fallback, and argument translation tests.
use super::*;
use crate::{BackendConfig, ProviderConfig, ProviderFuture, SearchProvider, SearchStatus};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tinysearch_bus::{PresentationMode, ProviderRoute, SearchConfig, names::tools};

type Calls = Arc<Mutex<Vec<ExecuteToolRequest>>>;

/// Records every request and answers with a scripted outcome.
struct Scripted {
    calls: Calls,
    outcome: fn() -> Result<ExecuteToolResponse>,
}

impl SearchProvider for Scripted {
    fn execute<'a>(
        &'a self,
        _config: &'a ProviderConfig,
        _backend: &'a BackendConfig,
        request: &'a ExecuteToolRequest,
    ) -> ProviderFuture<'a> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(request.clone());
        }
        let outcome = (self.outcome)();
        Box::pin(async move { outcome })
    }
}

fn ok() -> Result<ExecuteToolResponse> {
    Ok(ExecuteToolResponse {
        provider: "ignored".into(),
        results: vec![],
        citations: vec![],
        answer: Some("answer".into()),
        status: SearchStatus::Ok,
        provider_data: None,
        role: None,
        fallback_from: vec![],
    })
}
fn broke() -> Result<ExecuteToolResponse> {
    Err(Error::InsufficientBalance)
}
fn throttled() -> Result<ExecuteToolResponse> {
    Err(Error::RateLimited)
}
fn down() -> Result<ExecuteToolResponse> {
    Err(Error::ProviderUnavailable("provider returned HTTP 503".into()))
}
fn rejected() -> Result<ExecuteToolResponse> {
    Err(Error::RejectedArguments("provider returned HTTP 400".into()))
}

struct Fixture {
    service: SearchService,
    calls: BTreeMap<&'static str, Calls>,
}

impl Fixture {
    fn calls(&self, provider: &str) -> Vec<ExecuteToolRequest> {
        self.calls
            .get(provider)
            .and_then(|calls| calls.lock().ok().map(|calls| calls.clone()))
            .unwrap_or_default()
    }
}

/// Exa, Gemini and TinyFish on the backend; Brave, Tavily and Deep Research
/// with direct keys. Each provider answers with its scripted outcome.
fn fixture(
    outcomes: &[(&'static str, fn() -> Result<ExecuteToolResponse>)],
    configure: impl FnOnce(&mut SearchConfig),
) -> Fixture {
    let mut config = SearchConfig::default();
    config.backend.credential = Some("session".into());
    for name in ["exa", "gemini", "tinyfish"] {
        config.providers.insert(
            name.into(),
            ProviderConfig {
                route: ProviderRoute::Backend,
                ..ProviderConfig::default()
            },
        );
    }
    for name in ["brave", "tavily", "gemini_deep_research"] {
        config.providers.insert(
            name.into(),
            ProviderConfig {
                credential: Some("key".into()),
                ..ProviderConfig::default()
            },
        );
    }
    configure(&mut config);
    let mut providers: BTreeMap<String, Arc<dyn SearchProvider>> = BTreeMap::new();
    let mut calls = BTreeMap::new();
    for name in [
        "exa",
        "gemini",
        "tinyfish",
        "brave",
        "tavily",
        "gemini_deep_research",
    ] {
        let outcome = outcomes
            .iter()
            .find(|(provider, _)| *provider == name)
            .map_or(ok as fn() -> _, |(_, outcome)| *outcome);
        let recorded = Calls::default();
        calls.insert(name, recorded.clone());
        providers.insert(
            name.into(),
            Arc::new(Scripted {
                calls: recorded,
                outcome,
            }),
        );
    }
    Fixture {
        service: SearchService::with_providers(config, providers),
        calls,
    }
}

fn call(name: &str, arguments: Value) -> ExecuteToolRequest {
    ExecuteToolRequest {
        name: name.into(),
        arguments,
    }
}

#[test]
fn roles_mode_lists_the_three_role_tools() {
    let fixture = fixture(&[], |_| {});
    let listed = fixture.service.list_tools().tools;
    assert_eq!(
        listed.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>(),
        [tools::WEB_SEARCH, tools::WEB_ANSWER, tools::WEB_CONTENTS]
    );
}

#[tokio::test]
async fn search_uses_the_first_provider_and_translates_arguments() -> Result<()> {
    let fixture = fixture(&[], |_| {});
    let response = fixture
        .service
        .execute_tool(call(tools::WEB_SEARCH, json!({"query":"rust","max_results":3})))
        .await?;
    assert_eq!(response.provider, "exa");
    assert_eq!(response.role, Some(Role::Search));
    assert!(response.fallback_from.is_empty());
    let sent = fixture.calls("exa");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].name, "exa_search");
    // The backend Exa search accepts only the query.
    assert_eq!(sent[0].arguments, json!({"query":"rust"}));
    Ok(())
}

#[tokio::test]
async fn search_falls_back_in_order_past_provider_side_failures() -> Result<()> {
    let fixture = fixture(&[("exa", broke), ("brave", throttled), ("tavily", down)], |_| {});
    let response = fixture
        .service
        .execute_tool(call(tools::WEB_SEARCH, json!({"query":"rust","max_results":4})))
        .await?;
    assert_eq!(response.provider, "tinyfish");
    assert_eq!(response.fallback_from, ["exa", "brave", "tavily"]);
    assert_eq!(
        fixture.calls("brave")[0].arguments,
        json!({"query":"rust","count":4})
    );
    assert_eq!(
        fixture.calls("tavily")[0].arguments,
        json!({"query":"rust","max_results":4})
    );
    assert_eq!(fixture.calls("tinyfish")[0].arguments, json!({"query":"rust"}));
    Ok(())
}

#[tokio::test]
async fn invalid_arguments_do_not_fall_back() {
    let fixture = fixture(&[("exa", rejected)], |_| {});
    let result = fixture
        .service
        .execute_tool(call(tools::WEB_SEARCH, json!({"query":"rust"})))
        .await;
    assert_eq!(
        result.err(),
        Some(Error::RejectedArguments("provider returned HTTP 400".into()))
    );
    assert!(fixture.calls("brave").is_empty());
}

#[tokio::test]
async fn exhausted_fallbacks_return_the_last_error() {
    let fixture = fixture(
        &[
            ("exa", broke),
            ("brave", broke),
            ("tavily", broke),
            ("tinyfish", throttled),
        ],
        |_| {},
    );
    let result = fixture
        .service
        .execute_tool(call(tools::WEB_SEARCH, json!({"query":"rust"})))
        .await;
    assert_eq!(result.err(), Some(Error::RateLimited));
}

#[tokio::test]
async fn explicit_provider_is_used_alone_without_fallback() -> Result<()> {
    let fixture = fixture(&[("brave", broke)], |_| {});
    let result = fixture
        .service
        .execute_tool(call(
            tools::WEB_SEARCH,
            json!({"query":"rust","provider":"brave"}),
        ))
        .await;
    assert_eq!(result.err(), Some(Error::InsufficientBalance));
    assert!(fixture.calls("exa").is_empty());
    assert!(fixture.calls("tavily").is_empty());

    let response = fixture
        .service
        .execute_tool(call(
            tools::WEB_SEARCH,
            json!({"query":"rust","provider":"tavily"}),
        ))
        .await?;
    assert_eq!(response.provider, "tavily");
    assert!(fixture.calls("exa").is_empty());
    Ok(())
}

#[tokio::test]
async fn role_arguments_are_validated_before_dispatch() {
    let fixture = fixture(&[], |_| {});
    for arguments in [
        json!({}),
        json!({"query":"rust","provider":"querit"}),
        json!({"query":"rust","max_results":50}),
    ] {
        assert_eq!(
            fixture
                .service
                .execute_tool(call(tools::WEB_SEARCH, arguments))
                .await
                .err(),
            Some(Error::InvalidArguments)
        );
    }
    assert_eq!(
        fixture
            .service
            .execute_tool(call(tools::WEB_SEARCH, json!({"query":"rust","extra":1})))
            .await
            .err(),
        Some(Error::UnsupportedArgument("extra".into()))
    );
    assert_eq!(
        fixture
            .service
            .execute_tool(call("exa_search", json!({"query":"rust"})))
            .await
            .err(),
        Some(Error::UnavailableTool("exa_search".into()))
    );
    assert!(fixture.calls("exa").is_empty());
}

#[tokio::test]
async fn quick_answers_skip_deep_research() -> Result<()> {
    let fixture = fixture(&[("gemini", down)], |_| {});
    let response = fixture
        .service
        .execute_tool(call(tools::WEB_ANSWER, json!({"query":"why"})))
        .await?;
    assert_eq!(response.provider, "exa");
    assert_eq!(response.role, Some(Role::Answer));
    assert_eq!(response.fallback_from, ["gemini"]);
    assert_eq!(fixture.calls("gemini")[0].name, "gemini_agentic_search");
    assert_eq!(fixture.calls("exa")[0].name, "exa_answer");
    assert_eq!(fixture.calls("exa")[0].arguments, json!({"query":"why"}));
    assert!(fixture.calls("gemini_deep_research").is_empty());
    Ok(())
}

#[tokio::test]
async fn deep_answers_prefer_deep_research_then_fall_back() -> Result<()> {
    let fixture = fixture(&[], |_| {});
    let response = fixture
        .service
        .execute_tool(call(tools::WEB_ANSWER, json!({"query":"why","depth":"deep"})))
        .await?;
    assert_eq!(response.provider, "gemini_deep_research");
    assert_eq!(
        fixture.calls("gemini_deep_research")[0].arguments,
        json!({"query":"why"})
    );

    let fixture = fixture_without_deep_research();
    let response = fixture
        .service
        .execute_tool(call(tools::WEB_ANSWER, json!({"query":"why","depth":"deep"})))
        .await?;
    assert_eq!(response.provider, "gemini");
    Ok(())
}

fn fixture_without_deep_research() -> Fixture {
    fixture(&[], |config| {
        config.providers.remove("gemini_deep_research");
    })
}

#[tokio::test]
async fn contents_translate_urls_per_provider() -> Result<()> {
    let fixture = fixture(&[("exa", down), ("tavily", down)], |_| {});
    let response = fixture
        .service
        .execute_tool(call(
            tools::WEB_CONTENTS,
            json!({"urls":["https://example.test"],"query":"pricing"}),
        ))
        .await?;
    assert_eq!(response.provider, "tinyfish");
    assert_eq!(response.role, Some(Role::Contents));
    assert_eq!(response.fallback_from, ["exa", "tavily"]);
    assert_eq!(
        fixture.calls("exa")[0].arguments,
        json!({"urls":["https://example.test"],"query":"pricing"})
    );
    assert_eq!(fixture.calls("tavily")[0].name, "tavily_extract");
    assert_eq!(
        fixture.calls("tavily")[0].arguments,
        json!({"urls":["https://example.test"]})
    );
    assert_eq!(
        fixture.calls("tinyfish")[0].arguments,
        json!({"urls":["https://example.test"],"format":"markdown"})
    );
    Ok(())
}

#[tokio::test]
async fn configured_role_order_drives_dispatch() -> Result<()> {
    let fixture = fixture(&[], |config| {
        config
            .presentation
            .roles
            .insert(Role::Search, vec!["tavily".into(), "exa".into()]);
    });
    let response = fixture
        .service
        .execute_tool(call(tools::WEB_SEARCH, json!({"query":"rust"})))
        .await?;
    assert_eq!(response.provider, "tavily");
    Ok(())
}

#[tokio::test]
async fn legacy_modes_ignore_role_tools() {
    let fixture = fixture(&[], |config| {
        config.presentation.mode = PresentationMode::AllTools;
    });
    assert_eq!(
        fixture
            .service
            .execute_tool(call(tools::WEB_SEARCH, json!({"query":"rust"})))
            .await
            .err(),
        Some(Error::UnavailableTool(tools::WEB_SEARCH.into()))
    );
}

#[test]
fn answer_order_honours_depth() {
    let usable = vec![
        "gemini".to_owned(),
        DEEP_RESEARCH.to_owned(),
        "exa".to_owned(),
    ];
    let quick = Map::new();
    assert_eq!(answer_order(usable.clone(), &quick), ["gemini", "exa"]);
    let mut deep = Map::new();
    deep.insert("depth".into(), json!("deep"));
    assert_eq!(
        answer_order(usable, &deep),
        [DEEP_RESEARCH, "gemini", "exa"]
    );
    assert_eq!(
        answer_order(vec![DEEP_RESEARCH.to_owned()], &quick),
        [DEEP_RESEARCH]
    );
}
