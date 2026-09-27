//! In-memory `TinyBus` integration tests.
use super::{SearchBusService, setup};
use crate::{SearchConfig, SearchService};
use std::{collections::BTreeMap, sync::Arc};
use tinybus::{Connection, Interface, broker::Broker, transport::memory::MemoryBus};
use tinysearch_bus::{ExecuteToolRequest, ExecuteToolResponse, ListToolsResponse, errors, names};

#[test]
fn declared_methods_match_contract() {
    let methods = SearchBusService(Arc::new(SearchService::with_providers(
        SearchConfig::default(),
        BTreeMap::new(),
    )))
    .members()
    .into_iter()
    .map(|member| member.to_string())
    .collect::<Vec<_>>();
    assert_eq!(methods, names::METHODS);
}

#[test]
fn served_interface_matches_contract() {
    let service = SearchBusService(Arc::new(SearchService::with_providers(
        SearchConfig::default(),
        BTreeMap::new(),
    )));
    assert_eq!(service.name().to_string(), names::INTERFACE);
}

#[tokio::test]
async fn empty_configuration_serves_no_tools_and_rejects_execution() -> tinybus::Result<()> {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let server = Connection::connect(bus.connect().await?).await?;
    setup(server.clone(), SearchConfig::default()).await?;
    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let listed: ListToolsResponse = proxy.call(names::methods::LIST_TOOLS, ()).await?;
    assert!(listed.tools.is_empty());
    let result = proxy
        .call::<ExecuteToolResponse>(
            names::methods::EXECUTE_TOOL,
            (ExecuteToolRequest {
                name: "search".into(),
                arguments: serde_json::json!({"query":"test"}),
            },),
        )
        .await;
    assert!(result.is_err());
    Ok(())
}

#[test]
fn builtins_discover_and_reinitialize_from_private_configuration() {
    let mut config = SearchConfig::default();
    config.backend.base_url = Some("http://127.0.0.1:1".into());
    config.backend.credential = Some("private".into());
    config.providers.insert(
        "tinyfish".into(),
        crate::ProviderConfig {
            route: crate::ProviderRoute::Backend,
            ..crate::ProviderConfig::default()
        },
    );
    let service = SearchService::with_providers(config.clone(), crate::provider::builtins());
    let tools = service.list_tools().tools;
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        [names::tools::WEB_SEARCH, names::tools::WEB_CONTENTS]
    );
    config.presentation.mode = crate::PresentationMode::AllTools;
    let all = SearchService::with_providers(config.clone(), crate::provider::builtins());
    assert!(
        all.list_tools()
            .tools
            .iter()
            .any(|tool| tool.name == "tinyfish_agent_run")
    );
    if let Some(provider) = config.providers.get_mut("tinyfish") {
        provider.enabled = false;
    }
    let refreshed = SearchService::with_providers(config.clone(), crate::provider::builtins());
    assert!(refreshed.list_tools().tools.is_empty());
    config.enabled = false;
    let disabled = SearchService::with_providers(config, crate::provider::builtins());
    assert!(disabled.list_tools().tools.is_empty());
}

#[tokio::test]
async fn classified_failures_cross_the_bus_with_their_code() -> tinybus::Result<()> {
    let mut config = SearchConfig::default();
    // A local listener that closes every connection without answering, so
    // the backend call deterministically fails in transport.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let refuser = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });
    config.backend.base_url = Some(format!("http://127.0.0.1:{port}"));
    config.backend.credential = Some("private".into());
    config.providers.insert(
        "tinyfish".into(),
        crate::ProviderConfig {
            route: crate::ProviderRoute::Backend,
            ..crate::ProviderConfig::default()
        },
    );
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let server = Connection::connect(bus.connect().await?).await?;
    setup(server.clone(), config).await?;
    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let call = |arguments: serde_json::Value| {
        proxy.call::<ExecuteToolResponse>(
            names::methods::EXECUTE_TOOL,
            (ExecuteToolRequest {
                name: names::tools::WEB_SEARCH.into(),
                arguments,
            },),
        )
    };
    let unavailable = call(serde_json::json!({"query":"test"}))
        .await
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert_eq!(
        errors::code_of(&unavailable),
        Some(errors::UNAVAILABLE),
        "{unavailable}"
    );
    let invalid = call(serde_json::json!({"max_results":3}))
        .await
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert_eq!(
        errors::code_of(&invalid),
        Some(errors::INVALID_ARGUMENTS),
        "{invalid}"
    );
    refuser.abort();
    Ok(())
}
