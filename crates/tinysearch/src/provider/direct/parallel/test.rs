//! Direct Parallel request mapping, async resume, and catalog tests.
use super::*;
use crate::{BackendConfig, PresentationMode, ProviderRoute, SearchConfig, SearchService};
use super::super::super::builtins;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn mock(
    responses: Vec<(u16, Value)>,
) -> std::io::Result<(
    String,
    tokio::task::JoinHandle<std::io::Result<Vec<String>>>,
)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().await?;
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let n = stream.read(&mut chunk).await?;
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]);
                    let length = header
                        .lines()
                        .find_map(|s| {
                            s.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|v| v.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8_lossy(&bytes).into_owned());
            let body = body.to_string();
            stream.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
        }
        Ok(requests)
    });
    Ok((url, task))
}
fn request(name: &str, arguments: Value) -> ExecuteToolRequest {
    ExecuteToolRequest {
        name: name.into(),
        arguments,
    }
}
fn config(base_url: String) -> ProviderConfig {
    ProviderConfig {
        route: ProviderRoute::Direct,
        base_url: Some(base_url),
        credential: Some("secret-key".into()),
        ..ProviderConfig::default()
    }
}

#[tokio::test]
async fn search_maps_current_schema_and_bounds_results()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let results: Vec<Value> = (0..25).map(|i| json!({"url":format!("https://example.org/{i}"),"title":"T","publish_date":"2026-09-25","excerpts":["x".repeat(2000)]})).collect();
    let (url, server) = mock(vec![(
        200,
        json!({"results":results,"search_id":"search_1"}),
    )])
    .await?;
    let response = run(&Client::new(), &config(url), &request("parallel_search",json!({"objective":"Find","search_queries":["find this"],"mode":"advanced","num_results":5,"max_characters_per_excerpt":500}))).await?;
    let sent = server.await??.remove(0);
    assert!(sent.starts_with("POST /v1/search "));
    assert!(sent.to_ascii_lowercase().contains("x-api-key: secret-key"));
    let body: Value =
        serde_json::from_str(sent.split("\r\n\r\n").nth(1).ok_or("missing HTTP body")?)?;
    assert_eq!(
        body,
        json!({"objective":"Find","search_queries":["find this"],"mode":"advanced","advanced_settings":{"max_results":5,"excerpt_settings":{"max_chars_per_result":500}}})
    );
    assert_eq!(response.results.len(), 20);
    assert_eq!(
        response.results[0]
            .snippet
            .as_ref()
            .ok_or("missing snippet")?
            .len(),
        1200
    );
    assert_eq!(response.results[0].published.as_deref(), Some("2026-09-25"));
    assert_eq!(response.citations.len(), 20);
    Ok(())
}

#[tokio::test]
async fn extract_and_chat_use_direct_paths()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let (url, server) = mock(vec![(
        200,
        json!({"results":[{"url":"https://example.org","full_content":"body"}]}),
    )])
    .await?;
    let response = run(
        &Client::new(),
        &config(url),
        &request(
            "parallel_extract",
            json!({"urls":["https://example.org"],"full_content":true}),
        ),
    )
    .await?;
    let sent = server.await??.remove(0);
    assert!(sent.starts_with("POST /v1/extract "));
    assert!(sent.contains("\"advanced_settings\":{\"full_content\":true}"));
    assert_eq!(response.results[0].snippet.as_deref(), Some("body"));
    let (url, server) = mock(vec![(
        200,
        json!({"choices":[{"message":{"content":"answer"}}]}),
    )])
    .await?;
    let response = run(
        &Client::new(),
        &config(url),
        &request(
            "parallel_chat",
            json!({"model":"lite","messages":[{"role":"user","content":"hello"}]}),
        ),
    )
    .await?;
    let sent = server.await??.remove(0);
    assert!(sent.starts_with("POST /v1beta/chat/completions "));
    assert_eq!(response.answer.as_deref(), Some("answer"));
    Ok(())
}

#[tokio::test]
async fn task_creation_and_resume_are_bounded()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let (url, server) = mock(vec![(202, json!({"run_id":"trun_1","status":"queued"}))]).await?;
    let response = run(&Client::new(), &config(url), &request("parallel_enrich",json!({"input":{"company":"Example"},"processor":"base","output_schema":{"type":"object","properties":{"name":{"type":"string"}}}}))).await?;
    let sent = server.await??.remove(0);
    assert!(sent.starts_with("POST /v1/tasks/runs "));
    assert!(sent.contains("\"task_spec\":{\"output_schema\":{\"json_schema\":"));
    assert_eq!(response.status, SearchStatus::InProgress);
    assert_eq!(
        response.provider_data.ok_or("missing provider data")?["run_id"],
        "trun_1"
    );
    let (url, server) = mock(vec![(200,json!({"run_id":"trun_1","status":"completed"})),(200,json!({"run":{"run_id":"trun_1"},"output":{"type":"json","content":{"name":"Example"},"basis":[]}}))]).await?;
    let response = run(
        &Client::new(),
        &config(url),
        &request("parallel_enrich_status", json!({"run_id":"trun_1"})),
    )
    .await?;
    let sent = server.await??;
    assert!(sent[0].starts_with("GET /v1/tasks/runs/trun_1 "));
    assert!(sent[1].starts_with("GET /v1/tasks/runs/trun_1/result "));
    assert_eq!(response.status, SearchStatus::Ok);
    assert!(response.answer.ok_or("missing answer")?.contains("Example"));
    Ok(())
}

#[tokio::test]
async fn completed_task_preserves_field_basis_citations()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let (url, server) = mock(vec![
        (200, json!({"run_id":"trun_1","status":"completed"})),
        (200, json!({"run":{"run_id":"trun_1"},"output":{"type":"json","content":{"name":"Example"},"basis":[{"field":"name","reasoning":"Company site","citations":[{"title":"About Example","url":"https://example.org/about","excerpts":["Company profile"]}]}]}})),
    ]).await?;
    let response = run(
        &Client::new(),
        &config(url),
        &request("parallel_research_status", json!({"run_id":"trun_1"})),
    )
    .await?;
    server.await??;
    assert_eq!(response.citations.len(), 1);
    assert_eq!(response.citations[0].url, "https://example.org/about");
    assert_eq!(
        response.citations[0].title.as_deref(),
        Some("About Example")
    );
    Ok(())
}

#[test]
fn dataset_conditions_require_descriptions() {
    let request = request(
        "parallel_dataset",
        json!({"objective":"companies","entity_type":"company","match_conditions":[{"name":"is_public"}]}),
    );
    assert!(matches!(
        prepare(&ProviderConfig::default(), &request),
        Err(Error::InvalidArguments)
    ));
}

#[test]
fn async_http_timeout_is_capped_without_changing_sync_calls() {
    let config = ProviderConfig {
        timeout_secs: Some(1800),
        ..ProviderConfig::default()
    };
    assert_eq!(
        request_timeout(&config, Kind::Task),
        Duration::from_secs(35)
    );
    assert_eq!(
        request_timeout(&config, Kind::FindAll),
        Duration::from_secs(35)
    );
    assert_eq!(
        request_timeout(&config, Kind::Sync),
        Duration::from_secs(1800)
    );
}

#[test]
fn configured_search_count_is_clamped_to_parallel_api_range()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    for (configured, expected) in [(0, 1), (51, 50), (u64::MAX, 50)] {
        let config = ProviderConfig {
            max_results: Some(configured),
            ..ProviderConfig::default()
        };
        let (_, _, body, _) = prepare(
            &config,
            &request(
                "parallel_search",
                json!({"objective":"Find","search_queries":["find"]}),
            ),
        )?;
        assert_eq!(
            body.ok_or("missing request body")?["advanced_settings"]["max_results"],
            expected
        );
    }
    Ok(())
}

#[tokio::test]
async fn dataset_creation_and_resume_normalize_candidates()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let (url, server) = mock(vec![(
        200,
        json!({"findall_id":"findall_1","status":{"status":"running"}}),
    )])
    .await?;
    let response = run(&Client::new(), &config(url), &request("parallel_dataset",json!({"objective":"companies","entity_type":"company","match_conditions":[{"name":"is_public","description":"Company is publicly listed"}]}))).await?;
    let sent = server.await??.remove(0);
    assert!(sent.starts_with("POST /v1beta/findall/runs "));
    assert!(sent.contains("\"generator\":\"base\""));
    assert!(sent.contains("\"match_limit\":100"));
    assert_eq!(response.status, SearchStatus::InProgress);
    assert_eq!(
        response.provider_data.ok_or("missing provider data")?["findall_id"],
        "findall_1"
    );
    let mut candidates: Vec<Value> = (0..20).map(|i| json!({"candidate_id":format!("candidate_{i}"),"name":"Unmatched","url":format!("https://excluded.org/{i}"),"match_status":"unmatched"})).collect();
    candidates.extend([
        json!({"candidate_id":"candidate_generated","name":"Generated","url":"https://generated.org","match_status":"generated"}),
        json!({"candidate_id":"candidate_discarded","name":"Discarded","url":"https://discarded.org","match_status":"discarded"}),
        json!({"candidate_id":"candidate_matched","name":"Example","url":"https://example.org","match_status":"matched","output":{"public":true},"basis":[{"field":"public","reasoning":"Official listing","citations":[{"title":"Exchange listing","url":"https://exchange.org/example","excerpts":["Listed"]}]}]})
    ]);
    let (url, server) = mock(vec![
        (
            200,
            json!({"findall_id":"findall_1","status":{"status":"completed"}}),
        ),
        (200, json!({"candidates":candidates})),
    ])
    .await?;
    let response = run(
        &Client::new(),
        &config(url),
        &request("parallel_dataset_status", json!({"findall_id":"findall_1"})),
    )
    .await?;
    let sent = server.await??;
    assert!(sent[0].starts_with("GET /v1beta/findall/runs/findall_1 "));
    assert!(sent[1].starts_with("GET /v1beta/findall/runs/findall_1/result "));
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].url, "https://example.org");
    assert!(
        response
            .citations
            .iter()
            .any(|citation| citation.url == "https://exchange.org/example")
    );
    assert_eq!(response.status, SearchStatus::Ok);
    Ok(())
}

#[tokio::test]
async fn errors_do_not_expose_secret_or_response_body()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let (url, server) = mock(vec![(401, json!({"error":"secret-key private query"}))]).await?;
    let Err(error) = run(
        &Client::new(),
        &config(url),
        &request(
            "parallel_search",
            json!({"objective":"private query","search_queries":["private query"]}),
        ),
    )
    .await
    else {
        return Err("expected provider failure".into());
    };
    server.await??;
    assert!(!error.to_string().contains("secret-key"));
    assert!(!error.to_string().contains("private query"));
    assert!(error.to_string().contains("401"));
    assert!(
        prepare(
            &ProviderConfig::default(),
            &request("parallel_research_status", json!({"run_id":"../../bad"}))
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn all_tools_catalog_is_direct_only() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut config = SearchConfig {
        backend: BackendConfig {
            credential: Some("backend".into()),
            ..BackendConfig::default()
        },
        ..SearchConfig::default()
    };
    config.presentation.mode = PresentationMode::AllTools;
    config.providers.insert(
        "parallel".into(),
        ProviderConfig {
            route: ProviderRoute::Direct,
            credential: Some("direct".into()),
            ..ProviderConfig::default()
        },
    );
    let tools = SearchService::with_providers(config.clone(), builtins())
        .list_tools()
        .tools;
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "parallel_search",
            "parallel_extract",
            "parallel_chat",
            "parallel_research",
            "parallel_enrich",
            "parallel_dataset",
            "parallel_research_status",
            "parallel_enrich_status",
            "parallel_dataset_status",
        ]
    );
    let search = tools
        .iter()
        .find(|t| t.name == "parallel_search")
        .ok_or("missing search tool")?;
    assert!(
        search.parameters["properties"]["mode"]["enum"]
            .as_array()
            .ok_or("missing mode enum")?
            .contains(&json!("advanced"))
    );
    assert!(
        tools
            .iter()
            .find(|t| t.name == "parallel_extract")
            .ok_or("missing extract tool")?
            .parameters["properties"]
            .get("excerpts")
            .is_none()
    );
    // A backend route never makes Parallel usable, even with a backend
    // credential: there is no managed Parallel.
    config
        .providers
        .get_mut("parallel")
        .ok_or("missing provider")?
        .route = ProviderRoute::Backend;
    let service = SearchService::with_providers(config, builtins());
    assert!(service.list_tools().tools.is_empty());
    Ok(())
}

#[tokio::test]
async fn backend_route_is_refused_without_any_request()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let backend = ProviderConfig {
        route: ProviderRoute::Backend,
        credential: Some("secret-key".into()),
        ..ProviderConfig::default()
    };
    let error = super::super::run(
        &Client::new(),
        "parallel",
        &backend,
        &request(
            "parallel_search",
            json!({"objective":"Find","search_queries":["find"]}),
        ),
    )
    .await
    .err()
    .ok_or("backend route must fail")?;
    assert!(error.to_string().contains("direct route"));
    Ok(())
}

#[tokio::test]
async fn missing_credential_and_unknown_operations_fail_closed()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let keyless = ProviderConfig {
        credential: Some("  ".into()),
        ..ProviderConfig::default()
    };
    let error = run(
        &Client::new(),
        &keyless,
        &request("parallel_search", json!({})),
    )
    .await
    .err()
    .ok_or("keyless call must fail")?;
    assert!(error.to_string().contains("credential"));
    assert!(prepare(&ProviderConfig::default(), &request("parallel_unknown", json!({}))).is_err());
    assert!(matches!(
        prepare(
            &ProviderConfig::default(),
            &request("parallel_extract", json!({"urls":["https://a.test"],"excerpts":false}))
        ),
        Err(Error::Provider(_))
    ));
    Ok(())
}
