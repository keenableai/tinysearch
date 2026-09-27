use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

async fn mock(
    status: u16,
    response: Value,
) -> TestResult<(String, tokio::task::JoinHandle<TestResult<String>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut data = vec![0; 65536];
        let mut used = 0;
        loop {
            let read = stream.read(&mut data[used..]).await?;
            if read == 0 {
                break;
            }
            used += read;
            if let Some(end) = data[..used].windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&data[..end + 4]);
                let length: usize = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|v| v.trim().parse().ok())
                    })
                    .unwrap_or(0);
                if used >= end + 4 + length {
                    break;
                }
            }
        }
        let request = String::from_utf8_lossy(&data[..used]).into_owned();
        let body = response.to_string();
        let reply = format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(reply.as_bytes()).await?;
        Ok(request)
    });
    Ok((address, task))
}
async fn mock_sequence(
    responses: Vec<Value>,
) -> TestResult<(String, tokio::task::JoinHandle<TestResult<Vec<String>>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = listener.accept().await?;
            let mut data = vec![0; 8192];
            let used = stream.read(&mut data).await?;
            requests.push(String::from_utf8_lossy(&data[..used]).into_owned());
            let body = response.to_string();
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).await?;
        }
        Ok(requests)
    });
    Ok((address, task))
}
fn request(name: &str, arguments: Value) -> ExecuteToolRequest {
    ExecuteToolRequest {
        name: name.into(),
        arguments,
    }
}
fn backend(base_url: String, auth_mode: BackendAuthMode) -> BackendConfig {
    BackendConfig {
        base_url: Some(base_url),
        credential: Some("secret-token".into()),
        sdk_name: Some("openhuman".into()),
        auth_mode,
    }
}

#[test]
fn normalization_fills_limit_with_valid_results_after_invalid_entries() {
    let mut items = vec![json!({"title":"missing URL"}); 20];
    items.extend(
        (0..25).map(
            |n| json!({"url":format!("https://example.test/{n}"),"title":format!("Result {n}")}),
        ),
    );
    let response = normalize("tavily", "tavily_search", &json!({"results":items}));
    assert_eq!(response.results.len(), MAX_RESULTS);
    assert_eq!(response.results[0].url, "https://example.test/0");
    assert_eq!(response.results[19].url, "https://example.test/19");
    assert_eq!(response.citations.len(), MAX_RESULTS);
}

#[test]
fn provider_urls_reject_credentials_queries_and_non_http_schemes() -> TestResult<()> {
    for base in [
        "not a url",
        "ftp://example.test",
        "https://user@example.test",
        "https://example.test/?key=secret",
        "https://example.test/#fragment",
    ] {
        assert_eq!(
            direct_url(Some(base), "https://fallback.test", "/search")
                .err()
                .ok_or("expected provider error")?,
            Error::Provider("invalid provider URL".into())
        );
    }
    assert_eq!(
        direct_url(None, "https://example.test/root/", "/search")?,
        "https://example.test/root/search"
    );
    assert_eq!(
        backend_url(&BackendConfig::default(), "/search")
            .err()
            .ok_or("expected provider error")?,
        Error::Provider("backend URL unavailable".into())
    );
    Ok(())
}

#[tokio::test]
async fn backend_reports_rejected_failed_and_running_tasks() -> TestResult<()> {
    for (payload, expected_error, expected_status) in [
        (
            json!({"success":false,"data":{"secret":"body"}}),
            Some(Error::Provider("backend rejected provider request".into())),
            None,
        ),
        (
            json!({"success":true,"data":{"status":{"state":"failed"}}}),
            Some(Error::Provider("provider task failed".into())),
            None,
        ),
        (
            json!({"success":true,"data":{"status":"running"}}),
            None,
            Some(SearchStatus::InProgress),
        ),
    ] {
        let (url, server) = mock(200, payload).await?;
        let provider = BuiltinProvider {
            name: "tinyfish",
            client: Client::new(),
        };
        let result = provider
            .run(
                &ProviderConfig {
                    route: ProviderRoute::Backend,
                    ..ProviderConfig::default()
                },
                &backend(url, BackendAuthMode::Session),
                &request("tinyfish_search", json!({"query":"test"})),
            )
            .await;
        server.await??;
        assert!(expected_error.is_some() ^ expected_status.is_some());
        match (expected_error, expected_status) {
            (Some(error), None) => {
                assert_eq!(result.err().ok_or("expected provider error")?, error);
            }
            (None, Some(status)) => assert_eq!(result?.status, status),
            _ => {}
        }
    }
    Ok(())
}

#[tokio::test]
async fn provider_rejects_unknown_provider_tool_and_invalid_arguments() -> TestResult<()> {
    let config = ProviderConfig::default();
    let backend = BackendConfig::default();
    for (name, request_name, arguments, expected) in [
        (
            "unknown",
            "search",
            json!({}),
            Error::UnavailableProvider("unknown".into()),
        ),
        (
            "gemini",
            "wrong",
            json!({"query":"test"}),
            Error::UnavailableTool("wrong".into()),
        ),
        (
            "gemini",
            "gemini_agentic_search",
            json!({"query":" "}),
            Error::InvalidArguments,
        ),
        (
            "gemini",
            "gemini_agentic_search",
            json!({"query":"test","model":"bad/model"}),
            Error::InvalidArguments,
        ),
        (
            "gemini_deep_research",
            "wrong",
            json!({}),
            Error::UnavailableTool("wrong".into()),
        ),
    ] {
        let provider = BuiltinProvider {
            name,
            client: Client::new(),
        };
        assert_eq!(
            provider
                .run(&config, &backend, &request(request_name, arguments))
                .await
                .err()
                .ok_or("expected provider error")?,
            expected,
            "{name}/{request_name}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn provider_rejects_incompatible_routes_and_missing_backend_url() -> TestResult<()> {
    let backend = BackendConfig::default();
    let provider = BuiltinProvider {
        name: "tinyfish",
        client: Client::new(),
    };
    assert_eq!(
        provider
            .run(
                &ProviderConfig::default(),
                &backend,
                &request("tinyfish_search", json!({"query":"test"}))
            )
            .await
            .err()
            .ok_or("expected provider error")?,
        Error::Provider("this provider requires the backend route".into())
    );
    let backend_route = ProviderConfig {
        route: ProviderRoute::Backend,
        ..ProviderConfig::default()
    };
    let provider = BuiltinProvider {
        name: "gemini_deep_research",
        client: Client::new(),
    };
    assert_eq!(
        provider
            .run(
                &backend_route,
                &backend,
                &request("gemini_deep_research", json!({"query":"test"}))
            )
            .await
            .err()
            .ok_or("expected provider error")?,
        Error::Provider("Deep Research requires a direct Gemini route".into())
    );
    let provider = BuiltinProvider {
        name: "tinyfish",
        client: Client::new(),
    };
    assert_eq!(
        provider
            .run(
                &backend_route,
                &backend,
                &request("tinyfish_search", json!({"query":"test"}))
            )
            .await
            .err()
            .ok_or("expected provider error")?,
        Error::Provider("backend URL unavailable".into())
    );
    Ok(())
}

#[tokio::test]
async fn direct_deep_research_requires_credential_and_valid_interaction_id() -> TestResult<()> {
    let backend = BackendConfig::default();
    let provider = BuiltinProvider {
        name: "gemini_deep_research",
        client: Client::new(),
    };
    let direct = ProviderConfig {
        route: ProviderRoute::Direct,
        ..ProviderConfig::default()
    };
    assert_eq!(
        provider
            .run(
                &direct,
                &backend,
                &request("gemini_deep_research", json!({"query":"test"}))
            )
            .await
            .err()
            .ok_or("expected provider error")?,
        Error::Provider("Gemini credential unavailable".into())
    );
    let direct = ProviderConfig {
        credential: Some("key".into()),
        ..direct
    };
    assert_eq!(
        provider
            .run(
                &direct,
                &backend,
                &request("gemini_deep_research", json!({"interaction_id":"bad/id"}))
            )
            .await
            .err()
            .ok_or("expected provider error")?,
        Error::Provider("invalid Deep Research interaction id".into())
    );
    Ok(())
}

#[tokio::test]
async fn backend_requires_credential() -> TestResult<()> {
    let provider = BuiltinProvider {
        name: "tinyfish",
        client: Client::new(),
    };
    let mut backend = backend("http://127.0.0.1:1".into(), BackendAuthMode::Session);
    backend.credential = None;
    assert_eq!(
        provider
            .run(
                &ProviderConfig {
                    route: ProviderRoute::Backend,
                    ..ProviderConfig::default()
                },
                &backend,
                &request("tinyfish_search", json!({"query":"test"}))
            )
            .await
            .err()
            .ok_or("expected provider error")?,
        Error::Provider("backend credential unavailable".into())
    );
    Ok(())
}

fn sent_body(sent: &str) -> TestResult<Value> {
    let (_, body) = sent.split_once("\r\n\r\n").ok_or("request has no body")?;
    Ok(serde_json::from_str(body)?)
}

fn backend_route() -> ProviderConfig {
    ProviderConfig {
        route: ProviderRoute::Backend,
        ..ProviderConfig::default()
    }
}

#[tokio::test]
async fn exa_backend_search_sends_objective_and_queries_and_bounds_results() -> TestResult<()> {
    let items: Vec<Value> = (0..30)
        .map(|i| {
            json!({"url":format!("https://site/{i}"),"title":"t","publish_date":"2026-01-02","excerpts":["x".repeat(1500)]})
        })
        .collect();
    let (url, server) = mock(
        200,
        json!({"success":true,"data":{"searchId":"req-1","results":items,"costUsd":0.1}}),
    )
    .await?;
    let provider = BuiltinProvider {
        name: "exa",
        client: Client::new(),
    };
    let response = provider
        .run(
            &backend_route(),
            &backend(url, BackendAuthMode::ApiKey),
            &request("exa_search", json!({"query":"rust async"})),
        )
        .await?;
    let sent = server.await??;
    assert!(sent.starts_with("POST /agent-integrations/exa/search "));
    let headers = sent.to_ascii_lowercase();
    assert!(headers.contains("x-api-key: secret-token"));
    assert!(headers.contains("x-sdk-name: openhuman"));
    assert!(!headers.contains("authorization: bearer"));
    // Golden body: the backend route is strict and rejects `mode`.
    assert_eq!(
        sent_body(&sent)?,
        json!({"objective":"rust async","searchQueries":["rust async"]})
    );
    assert_eq!(response.provider, "exa");
    assert_eq!(response.results.len(), 20);
    assert_eq!(response.results[0].published.as_deref(), Some("2026-01-02"));
    assert_eq!(
        response.results[0]
            .snippet
            .as_ref()
            .ok_or("missing snippet")?
            .chars()
            .count(),
        1200
    );
    assert_eq!(response.citations.len(), 20);
    assert_eq!(
        response.provider_data.ok_or("missing provider data")?["searchId"],
        "req-1"
    );
    Ok(())
}

#[test]
fn exa_backend_routes_forward_exa_bodies() -> TestResult<()> {
    let cases = [
        (
            "exa_get_contents",
            json!({"urls":["https://a"],"query":"pricing"}),
            "/agent-integrations/exa/contents",
            json!({"urls":["https://a"],"text":true,"highlights":{"query":"pricing"}}),
        ),
        (
            "exa_find_similar",
            json!({"url":"https://a","max_results":3}),
            "/agent-integrations/exa/findSimilar",
            json!({"url":"https://a","numResults":3}),
        ),
        (
            "exa_answer",
            json!({"query":"why","include_text":true}),
            "/agent-integrations/exa/answer",
            json!({"query":"why","text":true}),
        ),
    ];
    for (name, arguments, path, body) in cases {
        assert_eq!(exa_request(&request(name, arguments))?, (path.into(), body));
    }
    assert_eq!(
        exa_request(&request("exa_search", json!({"query":" "}))).err(),
        Some(Error::InvalidArguments)
    );
    assert_eq!(
        exa_request(&request("exa_other", json!({}))).err(),
        Some(Error::UnavailableTool("exa_other".into()))
    );
    Ok(())
}

#[tokio::test]
async fn exa_backend_answer_normalizes_answer_and_citations() -> TestResult<()> {
    let (url, server) = mock(
        200,
        json!({"success":true,"data":{"answer":"Because.","citations":[{"id":"1","url":"https://a","title":"A"},{"id":"2","url":"https://b","title":null}],"costUsd":0.01}}),
    )
    .await?;
    let provider = BuiltinProvider {
        name: "exa",
        client: Client::new(),
    };
    let response = provider
        .run(
            &backend_route(),
            &backend(url, BackendAuthMode::Session),
            &request("exa_answer", json!({"query":"why"})),
        )
        .await?;
    let sent = server.await??;
    assert!(sent.starts_with("POST /agent-integrations/exa/answer "));
    assert_eq!(response.answer.as_deref(), Some("Because."));
    assert_eq!(response.status, SearchStatus::Ok);
    assert_eq!(
        response
            .citations
            .iter()
            .map(|citation| citation.url.as_str())
            .collect::<Vec<_>>(),
        ["https://a", "https://b"]
    );
    Ok(())
}

#[tokio::test]
async fn http_failures_map_to_stable_codes() -> TestResult<()> {
    for (status, body, expected) in [
        (402, json!({"success":false}), Error::InsufficientBalance),
        (
            400,
            json!({"success":false,"error":"Insufficient balance"}),
            Error::InsufficientBalance,
        ),
        (
            400,
            json!({"success":false,"error":"Insufficient budget","errorCode":"USER_INSUFFICIENT_CREDITS"}),
            Error::InsufficientBalance,
        ),
        (
            429,
            json!({"error":{"message":"slow down"}}),
            Error::RateLimited,
        ),
        (
            503,
            json!({}),
            Error::ProviderUnavailable("provider returned HTTP 503".into()),
        ),
        (
            500,
            json!({"success":false,"error":"Unable to estimate"}),
            Error::ProviderUnavailable("provider returned HTTP 500".into()),
        ),
        (
            400,
            json!({"success":false,"error":"Validation failed","errorCode":"VALIDATION_ERROR"}),
            Error::RejectedArguments("provider returned HTTP 400".into()),
        ),
        (
            422,
            json!({"message":"secret query"}),
            Error::RejectedArguments("provider returned HTTP 422".into()),
        ),
        (
            401,
            json!({}),
            Error::Provider("provider returned HTTP 401".into()),
        ),
    ] {
        let (url, server) = mock(status, body).await?;
        let provider = BuiltinProvider {
            name: "exa",
            client: Client::new(),
        };
        let error = provider
            .run(
                &backend_route(),
                &backend(url, BackendAuthMode::Session),
                &request("exa_search", json!({"query":"secret query"})),
            )
            .await
            .err()
            .ok_or("expected provider error")?;
        server.await??;
        assert!(!error.to_string().contains("secret query"));
        assert_eq!(error, expected, "HTTP {status}");
    }
    Ok(())
}

#[tokio::test]
async fn direct_provider_statuses_share_the_classification() -> TestResult<()> {
    for (status, expected) in [
        (402, Error::InsufficientBalance),
        (432, Error::InsufficientBalance),
        (429, Error::RateLimited),
        (
            502,
            Error::ProviderUnavailable("provider returned HTTP 502".into()),
        ),
    ] {
        let (url, server) = mock(status, json!({"detail":"private"})).await?;
        let error = direct::run(
            &Client::new(),
            "tavily",
            &ProviderConfig {
                base_url: Some(url),
                credential: Some("tavily-key".into()),
                ..ProviderConfig::default()
            },
            &request("tavily_search", json!({"query":"q"})),
        )
        .await
        .err()
        .ok_or("expected provider error")?;
        server.await??;
        assert_eq!(error, expected, "HTTP {status}");
    }
    Ok(())
}

#[tokio::test]
async fn unreachable_provider_is_unavailable() -> TestResult<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    drop(listener);
    let provider = BuiltinProvider {
        name: "exa",
        client: Client::new(),
    };
    let error = provider
        .run(
            &backend_route(),
            &backend(url, BackendAuthMode::Session),
            &request("exa_search", json!({"query":"q"})),
        )
        .await
        .err()
        .ok_or("expected provider error")?;
    assert_eq!(
        error,
        Error::ProviderUnavailable("provider transport failed".into())
    );
    Ok(())
}

#[tokio::test]
async fn gemini_grounding_normalizes_supported_chunks_first() -> TestResult<()> {
    let (url, server) = mock(
        200,
        json!({"success":true,"data":{"candidates":[{
            "content":{"parts":[{"text":"thinking","thought":true},{"text":"Rust is "},{"text":"fast."}]},
            "groundingMetadata":{
                "webSearchQueries":["rust speed"],
                "groundingChunks":[
                    {"web":{"uri":"https://unused","title":"unused.test"}},
                    {"web":{"uri":"https://second","title":"second.test"}},
                    {"web":{"uri":"https://first","title":"first.test"}},
                    {"retrievedContext":{"uri":"gs://private"}}
                ],
                "groundingSupports":[
                    {"segment":{"text":"Rust is"},"groundingChunkIndices":[2,9]},
                    {"segment":{"text":"fast."},"groundingChunkIndices":[1,2]}
                ]
            }
        }],"costUsd":0.002}}),
    )
    .await?;
    let provider = BuiltinProvider {
        name: "gemini",
        client: Client::new(),
    };
    let response = provider
        .run(
            &backend_route(),
            &backend(url, BackendAuthMode::Session),
            &request("gemini_agentic_search", json!({"query":"is rust fast"})),
        )
        .await?;
    let sent = server.await??;
    assert_eq!(
        sent_body(&sent)?,
        json!({"contents":[{"parts":[{"text":"is rust fast"}]}],"tools":[{"googleSearch":{}}]})
    );
    assert_eq!(response.answer.as_deref(), Some("Rust is fast."));
    assert_eq!(
        response
            .citations
            .iter()
            .map(|citation| (citation.url.as_str(), citation.title.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("https://first", Some("first.test")),
            ("https://second", Some("second.test")),
            ("https://unused", Some("unused.test"))
        ]
    );
    Ok(())
}

#[tokio::test]
async fn gemini_backend_uses_bearer_and_google_grounding() -> TestResult<()> {
    let (url, server) = mock(200, json!({"success":true,"data":{"candidates":[{"content":{"parts":[{"text":"answer"}]},"groundingMetadata":{"groundingChunks":[{"web":{"uri":"https://source","title":"Source"}}]}}]}})).await?;
    let provider = BuiltinProvider {
        name: "gemini",
        client: Client::new(),
    };
    let response = provider
        .run(
            &ProviderConfig {
                route: ProviderRoute::Backend,
                ..ProviderConfig::default()
            },
            &backend(url, BackendAuthMode::Session),
            &request("gemini_agentic_search", json!({"query":"question"})),
        )
        .await?;
    let sent = server.await??;
    assert!(
        sent.starts_with(
            "POST /agent-integrations/gemini/models/gemini-3.8-flash/generate-content "
        )
    );
    assert!(
        sent.to_ascii_lowercase()
            .contains("authorization: bearer secret-token")
    );
    assert!(sent.contains("\"googleSearch\":{}"));
    assert_eq!(response.answer.as_deref(), Some("answer"));
    assert_eq!(response.citations[0].url, "https://source");
    Ok(())
}

#[tokio::test]
async fn direct_gemini_keeps_backend_headers_off_request() -> TestResult<()> {
    let (url, server) = mock(
        200,
        json!({"candidates":[{"content":{"parts":[{"text":"answer"}]}}]}),
    )
    .await?;
    let provider = BuiltinProvider {
        name: "gemini",
        client: Client::new(),
    };
    let config = ProviderConfig {
        route: ProviderRoute::Direct,
        credential: Some("google-key".into()),
        base_url: Some(url),
        ..ProviderConfig::default()
    };
    let response = provider
        .run(
            &config,
            &backend("http://unused".into(), BackendAuthMode::ApiKey),
            &request("gemini_agentic_search", json!({"query":"question"})),
        )
        .await?;
    let sent = server.await??;
    assert!(
        sent.to_ascii_lowercase()
            .contains("x-goog-api-key: google-key")
    );
    assert!(!sent.to_ascii_lowercase().contains("x-sdk-name:"));
    assert!(!sent.to_ascii_lowercase().contains("x-api-key:"));
    assert!(sent.contains("\"google_search\":{}"));
    assert_eq!(response.answer.as_deref(), Some("answer"));
    Ok(())
}

#[tokio::test]
async fn upstream_error_does_not_echo_sensitive_body() -> TestResult<()> {
    let (url, server) = mock(400, json!({"message":"secret query"})).await?;
    let provider = BuiltinProvider {
        name: "tinyfish",
        client: Client::new(),
    };
    let error = provider
        .run(
            &ProviderConfig {
                route: ProviderRoute::Backend,
                ..ProviderConfig::default()
            },
            &backend(url, BackendAuthMode::Session),
            &request("tinyfish_search", json!({"query":"secret query"})),
        )
        .await
        .err()
        .ok_or("expected provider error")?;
    server.await??;
    assert_eq!(
        error.to_string(),
        "provider rejected the request arguments: provider returned HTTP 400"
    );
    Ok(())
}

#[test]
fn provider_request_mappings_preserve_legacy_knobs() -> TestResult<()> {
    let tinyfish_cases = [
        (
            "tinyfish_search",
            json!({"query":"test","page":3,"include_thumbnail":true}),
            "/agent-integrations/tinyfish/search",
            "page",
        ),
        (
            "tinyfish_fetch",
            json!({"urls":["https://a"],"format":"html","links":true}),
            "/agent-integrations/tinyfish/fetch",
            "links",
        ),
        (
            "tinyfish_agent_run",
            json!({"url":"https://a","goal":"find","proxy_country_code":"US","use_vault":true,"credential_item_ids":["id"]}),
            "/agent-integrations/tinyfish/agent/run",
            "proxy_config",
        ),
    ];
    for (name, arguments, path, field) in tinyfish_cases {
        let (actual_path, body) = tinyfish_request(&request(name, arguments))?;
        assert_eq!(actual_path, path);
        assert!(body.get(field).is_some(), "{name} missing {field}");
    }
    Ok(())
}

#[tokio::test]
async fn deep_research_completed_response_is_normalized() -> TestResult<()> {
    let (url, server) = mock(
        200,
        json!({"id":"abc_123","status":"completed","steps":[{"content":[{"text":"Report"}]}]}),
    )
    .await?;
    let provider = BuiltinProvider {
        name: "gemini_deep_research",
        client: Client::new(),
    };
    let config = ProviderConfig {
        route: ProviderRoute::Direct,
        credential: Some("google-key".into()),
        base_url: Some(url),
        ..ProviderConfig::default()
    };
    let response = provider
        .run(
            &config,
            &BackendConfig::default(),
            &request("gemini_deep_research", json!({"query":"research"})),
        )
        .await?;
    let sent = server.await??;
    assert!(sent.starts_with("POST /v1beta/interactions "));
    assert!(sent.contains("\"background\":true"));
    assert!(sent.contains("\"agent\":\"deep-research-preview-04-2026\""));
    assert_eq!(response.answer.as_deref(), Some("Report"));
    assert_eq!(response.status, SearchStatus::Ok);
    Ok(())
}

#[tokio::test]
async fn deep_research_last_poll_returns_completed_report() -> TestResult<()> {
    let (url, server) = mock_sequence(vec![
        json!({"id":"run_123","status":"in_progress"}),
        json!({"id":"run_123","status":"completed","steps":[{"content":[{"text":"Final report"}]}]}),
    ]).await?;
    let provider = BuiltinProvider {
        name: "gemini_deep_research",
        client: Client::new(),
    };
    let config = ProviderConfig {
        route: ProviderRoute::Direct,
        credential: Some("google-key".into()),
        base_url: Some(url),
        ..ProviderConfig::default()
    };
    let response = provider
        .run(
            &config,
            &BackendConfig::default(),
            &request(
                "gemini_deep_research",
                json!({"query":"research","max_poll_attempts":1}),
            ),
        )
        .await?;
    let requests = server.await??;
    assert!(requests[1].starts_with("GET /v1beta/interactions/run_123 "));
    assert_eq!(response.status, SearchStatus::Ok);
    assert_eq!(response.answer.as_deref(), Some("Final report"));
    Ok(())
}

#[tokio::test]
async fn deep_research_last_poll_redacts_failure() -> TestResult<()> {
    let (url, server) = mock_sequence(vec![
        json!({"id":"run_123","status":"in_progress"}),
        json!({"id":"run_123","status":"failed","error":"secret query"}),
    ])
    .await?;
    let provider = BuiltinProvider {
        name: "gemini_deep_research",
        client: Client::new(),
    };
    let config = ProviderConfig {
        route: ProviderRoute::Direct,
        credential: Some("google-key".into()),
        base_url: Some(url),
        ..ProviderConfig::default()
    };
    let error = provider
        .run(
            &config,
            &BackendConfig::default(),
            &request(
                "gemini_deep_research",
                json!({"query":"secret query","max_poll_attempts":1}),
            ),
        )
        .await
        .err()
        .ok_or("expected provider error")?;
    server.await??;
    assert_eq!(
        error.to_string(),
        "provider request failed: Deep Research task failed"
    );
    Ok(())
}

#[tokio::test]
async fn backend_failure_envelope_is_redacted() -> TestResult<()> {
    let (url, server) = mock(200, json!({"success":false,"message":"secret query"})).await?;
    let provider = BuiltinProvider {
        name: "exa",
        client: Client::new(),
    };
    let error = provider
        .run(
            &ProviderConfig {
                route: ProviderRoute::Backend,
                ..ProviderConfig::default()
            },
            &backend(url, BackendAuthMode::Session),
            &request("exa_search", json!({"query":"secret query"})),
        )
        .await
        .err()
        .ok_or("expected provider error")?;
    server.await??;
    assert_eq!(
        error.to_string(),
        "provider request failed: backend rejected provider request"
    );
    let (url, server) = mock(
        200,
        json!({"success":false,"errorCode":"USER_INSUFFICIENT_CREDITS"}),
    )
    .await?;
    let error = provider
        .run(
            &backend_route(),
            &backend(url, BackendAuthMode::Session),
            &request("exa_search", json!({"query":"q"})),
        )
        .await
        .err()
        .ok_or("expected provider error")?;
    server.await??;
    assert_eq!(error, Error::InsufficientBalance);
    Ok(())
}

#[tokio::test]
async fn deep_research_returns_resumable_in_progress_status() -> TestResult<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await?;
            let mut bytes = vec![0; 8192];
            let len = stream.read(&mut bytes).await?;
            requests.push(String::from_utf8_lossy(&bytes[..len]).to_string());
            let body = json!({"id":"run_123","status":"in_progress"}).to_string();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(requests)
    });
    let provider = BuiltinProvider {
        name: "gemini_deep_research",
        client: Client::new(),
    };
    let config = ProviderConfig {
        route: ProviderRoute::Direct,
        credential: Some("google-key".into()),
        base_url: Some(url),
        ..ProviderConfig::default()
    };
    let response = provider
        .run(
            &config,
            &BackendConfig::default(),
            &request(
                "gemini_deep_research",
                json!({"query":"research","max_poll_attempts":1}),
            ),
        )
        .await?;
    let requests = server.await??;
    assert!(requests[0].starts_with("POST /v1beta/interactions "));
    assert!(requests[1].starts_with("GET /v1beta/interactions/run_123 "));
    assert_eq!(response.status, SearchStatus::InProgress);
    assert_eq!(
        response.provider_data.ok_or("missing provider data")?["interaction_id"],
        "run_123"
    );
    Ok(())
}

#[tokio::test]
async fn exa_direct_operations_map_and_normalize() -> TestResult<()> {
    for (tool, args, path, body_field) in [
        (
            "exa_search",
            json!({"query":"rust","max_results":20,"include_highlights":true}),
            "/search",
            "\"contents\":{\"highlights\":true}",
        ),
        (
            "exa_find_similar",
            json!({"url":"https://seed.test","exclude_source_domain":true}),
            "/findSimilar",
            "\"excludeSourceDomain\":true",
        ),
        (
            "exa_get_contents",
            json!({"urls":["https://seed.test"],"include_summary":true}),
            "/contents",
            "\"summary\":true",
        ),
    ] {
        let (url, server) = mock(
            200,
            json!({"results":[{"url":"https://result.test","title":"Result","summary":"Summary"}]}),
        )
        .await?;
        let response = direct::run(
            &Client::new(),
            "exa",
            &ProviderConfig {
                base_url: Some(url),
                credential: Some("exa-secret".into()),
                ..ProviderConfig::default()
            },
            &request(tool, args),
        )
        .await?;
        let sent = server.await??;
        assert!(sent.starts_with(&format!("POST {path} ")));
        assert!(sent.to_ascii_lowercase().contains("x-api-key: exa-secret"));
        assert!(sent.contains(body_field));
        assert_eq!(response.results[0].snippet.as_deref(), Some("Summary"));
        assert_eq!(response.citations[0].url, "https://result.test");
    }
    Ok(())
}

#[tokio::test]
async fn brave_direct_operations_map_and_normalize() -> TestResult<()> {
    for (tool, path, payload) in [
        (
            "brave_web_search",
            "/web/search",
            json!({"web":{"results":[{"url":"https://web.test","title":"Web","description":"Excerpt"}]}}),
        ),
        (
            "brave_news_search",
            "/news/search",
            json!({"results":[{"url":"https://news.test","title":"News"}]}),
        ),
        (
            "brave_image_search",
            "/images/search",
            json!({"results":[{"url":"https://source.test/page","title":"Image","properties":{"url":"https://image.test"}}]}),
        ),
        (
            "brave_video_search",
            "/videos/search",
            json!({"results":[{"url":"https://video.test","title":"Video"}]}),
        ),
    ] {
        let (url, server) = mock(200, payload).await?;
        let response = direct::run(
            &Client::new(),
            "brave",
            &ProviderConfig {
                base_url: Some(url),
                credential: Some("brave-secret".into()),
                ..ProviderConfig::default()
            },
            &request(tool, json!({"query":"rust","count":20,"country":"us"})),
        )
        .await?;
        let sent = server.await??;
        assert!(sent.starts_with(&format!("GET {path}?")));
        assert!(sent.contains("count=20"));
        assert!(
            sent.to_ascii_lowercase()
                .contains("x-subscription-token: brave-secret")
        );
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.citations.len(), 1);
        if tool == "brave_image_search" {
            assert_eq!(response.results[0].url, "https://source.test/page");
            assert_eq!(response.citations[0].url, "https://source.test/page");
            assert_eq!(
                response.provider_data.ok_or("missing provider data")?["image_assets"][0]["image_url"],
                "https://image.test"
            );
        }
    }
    Ok(())
}

#[test]
fn querit_preserves_nested_filter_shapes() -> TestResult<()> {
    let prepared = direct::prepare(
        "querit",
        &ProviderConfig::default(),
        &request(
            "querit_search",
            json!({
                "query":"rust", "date":"2026-09-01", "countries":["japan"],
                "filters":{
                    "sites":["example.org"],
                    "time_range":{"date":"last_week","timezone":"UTC"},
                    "geo":{"countries":["france"],"region":"eu"},
                    "languages":["english"],
                    "custom":"keep"
                }
            }),
        ),
        "secret",
    )?;
    let body = prepared.3.ok_or("missing prepared request body")?;
    assert_eq!(
        body["filters"],
        json!({
            "sites":{"include":["example.org"]},
            "timeRange":{"date":"2026-09-01","timezone":"UTC"},
            "geo":{"countries":{"include":["japan"]},"region":"eu"},
            "languages":{"include":["english"]},
            "custom":"keep"
        })
    );
    Ok(())
}

#[test]
fn querit_ignores_malformed_top_level_site_filters() -> TestResult<()> {
    let prepared = direct::prepare(
        "querit",
        &ProviderConfig::default(),
        &request(
            "querit_search",
            json!({
                "query":"rust",
                "include_domains":"invalid",
                "exclude_domains":{"unexpected":"value"},
                "filters":{"sites":{"include":["example.org"],"exclude":["spam.org"]}}
            }),
        ),
        "secret",
    )?;
    assert_eq!(
        prepared.3.ok_or("missing prepared request body")?["filters"]["sites"],
        json!({"include":["example.org"],"exclude":["spam.org"]})
    );
    Ok(())
}

#[test]
fn direct_provider_config_controls_default_count_and_timeout() -> TestResult<()> {
    let config: ProviderConfig = serde_json::from_value(json!({
        "credential":"secret", "max_results":11, "timeout_secs":2
    }))?;
    let prepared = direct::prepare(
        "exa",
        &config,
        &request("exa_search", json!({"query":"rust"})),
        "secret",
    )?;
    assert_eq!(
        prepared.3.ok_or("missing prepared request body")?["numResults"],
        11
    );
    assert_eq!(
        direct::configured_timeout(&config, std::time::Duration::from_secs(35)),
        std::time::Duration::from_secs(2)
    );
    let long_timeout = ProviderConfig {
        timeout_secs: Some(1800),
        ..ProviderConfig::default()
    };
    assert_eq!(
        direct::configured_timeout(&long_timeout, std::time::Duration::from_secs(35)),
        std::time::Duration::from_secs(1800)
    );
    let explicit = direct::prepare(
        "exa",
        &config,
        &request("exa_search", json!({"query":"rust","max_results":4})),
        "secret",
    )?;
    assert_eq!(
        explicit.3.ok_or("missing prepared request body")?["numResults"],
        4
    );
    let legacy = direct::prepare(
        "exa",
        &ProviderConfig::default(),
        &request("exa_search", json!({"query":"rust"})),
        "secret",
    )?;
    assert_eq!(
        legacy.3.ok_or("missing prepared request body")?["numResults"],
        5
    );
    let brave = direct::prepare(
        "brave",
        &config,
        &request("brave_web_search", json!({"query":"rust"})),
        "secret",
    )?;
    assert!(brave.4.contains(&("count", "11".into())));
    let querit = direct::prepare(
        "querit",
        &config,
        &request("querit_search", json!({"query":"rust"})),
        "secret",
    )?;
    assert_eq!(
        querit.3.ok_or("missing prepared request body")?["count"],
        11
    );
    let querit_alias = direct::prepare(
        "querit",
        &config,
        &request("querit_search", json!({"query":"rust","count":3})),
        "secret",
    )?;
    assert_eq!(
        querit_alias.3.ok_or("missing prepared request body")?["count"],
        3
    );
    let tavily = direct::prepare(
        "tavily",
        &config,
        &request("tavily_search", json!({"query":"rust"})),
        "secret",
    )?;
    assert_eq!(
        tavily.3.ok_or("missing prepared request body")?["max_results"],
        11
    );
    let unbounded = ProviderConfig {
        timeout_secs: Some(u64::MAX),
        ..ProviderConfig::default()
    };
    assert_eq!(
        direct::configured_timeout(&unbounded, std::time::Duration::from_secs(35)),
        std::time::Duration::from_secs(u64::MAX)
    );
    Ok(())
}

#[tokio::test]
async fn querit_and_tavily_direct_map_auth_and_status() -> TestResult<()> {
    let (url, server) = mock(200,json!({"response_data":{"aiapi_res":{"error_code":0,"search_id":42,"results":{"result":[{"url":"https://querit.test","sentence":["One","two"]}]}}}})).await?;
    let result = direct::run(
        &Client::new(),
        "querit",
        &ProviderConfig {
            base_url: Some(url),
            credential: Some("q-secret".into()),
            ..ProviderConfig::default()
        },
        &request(
            "querit_search",
            json!({"query":"rust","countries":["united states"]}),
        ),
    )
    .await?;
    let sent = server.await??;
    assert!(sent.starts_with("POST /search "));
    assert!(
        sent.to_ascii_lowercase()
            .contains("authorization: bearer q-secret")
    );
    assert!(sent.contains("\"countries\":{\"include\":[\"united states\"]}"));
    assert_eq!(result.results[0].snippet.as_deref(), Some("One two"));
    assert_eq!(
        result.provider_data.ok_or("missing provider data")?["searchId"],
        42
    );

    for (tool, args, path, payload) in [
        (
            "tavily_search",
            json!({"query":"rust","include_answer":true,"topic":"news"}),
            "/search",
            json!({"answer":"Answer","results":[{"url":"https://tavily.test","content":"Excerpt"}]}),
        ),
        (
            "tavily_extract",
            json!({"urls":["https://tavily.test"],"extract_depth":"advanced"}),
            "/extract",
            json!({"results":[{"url":"https://tavily.test","raw_content":"Page text"}],"failed_results":[{"url":"https://failed.test"}]}),
        ),
    ] {
        let (url, server) = mock(200, payload).await?;
        let result = direct::run(
            &Client::new(),
            "tavily",
            &ProviderConfig {
                base_url: Some(url),
                credential: Some("t-secret".into()),
                ..ProviderConfig::default()
            },
            &request(tool, args),
        )
        .await?;
        let sent = server.await??;
        assert!(sent.starts_with(&format!("POST {path} ")));
        assert!(
            sent.to_ascii_lowercase()
                .contains("authorization: bearer t-secret")
        );
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.citations.len(), 1);
    }
    Ok(())
}

#[tokio::test]
async fn direct_rejects_backend_route_and_redacts_upstream_error() -> TestResult<()> {
    let request = request("exa_search", json!({"query":"private-query"}));
    let error = direct::run(
        &Client::new(),
        "exa",
        &ProviderConfig {
            route: ProviderRoute::Backend,
            ..ProviderConfig::default()
        },
        &request,
    )
    .await
    .err()
    .ok_or("expected provider error")?;
    assert!(error.to_string().contains("direct route"));
    let (url, server) = mock(403, json!({"error":"private-query exa-secret"})).await?;
    let error = direct::run(
        &Client::new(),
        "exa",
        &ProviderConfig {
            base_url: Some(url),
            credential: Some("exa-secret".into()),
            ..ProviderConfig::default()
        },
        &request,
    )
    .await
    .err()
    .ok_or("expected provider error")?;
    server.await??;
    assert!(error.to_string().contains("403"));
    assert!(!error.to_string().contains("private-query"));
    assert!(!error.to_string().contains("exa-secret"));
    Ok(())
}
