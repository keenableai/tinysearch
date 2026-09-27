use super::*;
use crate::{SearchConfig, SearchService};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

async fn mock(
    status: u16,
    body: Value,
) -> TestResult<(String, tokio::task::JoinHandle<TestResult<String>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut request = vec![0; 65536];
        let mut used = 0;
        loop {
            let n = stream.read(&mut request[used..]).await?;
            if n == 0 {
                break;
            }
            used += n;
            if let Some(end) = request[..used].windows(4).position(|w| w == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&request[..end + 4]);
                let length = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|s| s.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if used >= end + 4 + length {
                    break;
                }
            }
        }
        let sent = String::from_utf8_lossy(&request[..used]).to_string();
        let body = body.to_string();
        stream.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
        Ok(sent)
    });
    Ok((url, task))
}

fn request(name: &str, arguments: Value) -> ExecuteToolRequest {
    ExecuteToolRequest {
        name: name.into(),
        arguments,
    }
}

#[tokio::test]
async fn seltz_maps_options_key_and_result_limit() -> TestResult<()> {
    let (url, server) = mock(
        200,
        json!({"documents":[
        {"url":"https://one.test","title":" ","content":" First ","published_date":"2026-01-02"},
        {"url":"https://two.test","title":"Second","content":"Later"}]}),
    )
    .await?;
    let config = ProviderConfig {
        base_url: Some(format!("{url}/v1")),
        credential: Some("secret-key".into()),
        max_results: Some(1),
        timeout_secs: Some(3),
        ..Default::default()
    };
    let response = run(&Client::new(), "seltz", &config, &request("seltz_search", json!({"query":"private words","include_domains":["example.org"],"exclude_domains":["spam.org"],"from_date":"2026-01-01","to_date":"2026-02-01","scope":"news"}))).await?;
    let sent = server.await??;
    assert!(sent.starts_with("POST /v1/search "));
    assert!(sent.to_ascii_lowercase().contains("x-api-key: secret-key"));
    for field in [
        "\"max_results\":1",
        "\"include_domains\":[\"example.org\"]",
        "\"exclude_domains\":[\"spam.org\"]",
        "\"from_date\":\"2026-01-01\"",
        "\"to_date\":\"2026-02-01\"",
        "\"scope\":\"news\"",
    ] {
        assert!(sent.contains(field), "missing {field}");
    }
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].title, "Untitled");
    assert_eq!(response.results[0].snippet.as_deref(), Some(" First "));
    assert_eq!(response.results[0].published.as_deref(), Some("2026-01-02"));
    assert_eq!(response.citations[0].url, "https://one.test");
    Ok(())
}

#[test]
fn seltz_publication_is_bounded_and_blank_dates_are_omitted() -> TestResult<()> {
    let response = normalize(
        "seltz",
        "seltz_search",
        &seltz::unwrap(
            &json!({"documents":[
                {"url":"https://long.test","published_date":"x".repeat(150)},
                {"url":"https://blank.test","published_date":"  "}
            ]}),
            2,
        ),
    );
    assert_eq!(
        response.results[0]
            .published
            .as_ref()
            .ok_or("missing publication")?
            .len(),
        100
    );
    assert_eq!(response.results[1].published, None);
    Ok(())
}

#[tokio::test]
async fn searxng_maps_categories_language_and_normalizes_up_to_fifty() -> TestResult<()> {
    let mut items = vec![
        json!({"url":" ","title":"discard"}),
        json!({"url":"https://one.test","title":" ","content":" Content ","snippet":"fallback","engine":"engine-a"}),
        json!({"url":"https://two.test","snippet":"Snippet","engines":["engine-b"]}),
    ];
    items.extend((0..60).map(|i| json!({"url":format!("https://result.test/{i}")})));
    let (url, server) = mock(200, json!({"results":items})).await?;
    let config = ProviderConfig {
        base_url: Some(format!("{url}/search")),
        max_results: Some(50),
        default_language: Some("fr".into()),
        ..Default::default()
    };
    let response = run(
        &Client::new(),
        "searxng",
        &config,
        &request(
            "searxng_search",
            json!({"query":"  rust search  ","categories":["web","GENERAL","news","images"]}),
        ),
    )
    .await?;
    let sent = server.await??;
    assert!(sent.starts_with("GET /search?"));
    assert!(sent.contains("q=rust+search"));
    assert!(sent.contains("format=json"));
    assert!(sent.contains("categories=general%2Cnews%2Cimages"));
    assert!(sent.contains("language=fr"));
    assert!(!sent.to_ascii_lowercase().contains("authorization:"));
    assert_eq!(response.results.len(), 50);
    assert_eq!(response.results[0].title, "https://one.test");
    assert_eq!(response.results[0].snippet.as_deref(), Some("Content"));
    assert_eq!(
        response.provider_data.ok_or("missing provider data")?["sources"][1],
        "engine-b"
    );
    assert_eq!(response.citations.len(), 40);
    Ok(())
}

#[tokio::test]
async fn direct_errors_hide_query_key_and_response_body() -> TestResult<()> {
    let (url, server) = mock(403, json!({"message":"private words secret-key"})).await?;
    let error = run(
        &Client::new(),
        "seltz",
        &ProviderConfig {
            base_url: Some(url),
            credential: Some("secret-key".into()),
            ..Default::default()
        },
        &request("seltz_search", json!({"query":"private words"})),
    )
    .await
    .err()
    .ok_or("expected provider error")?;
    server.await??;
    assert!(error.to_string().contains("403"));
    assert!(!error.to_string().contains("private words"));
    assert!(!error.to_string().contains("secret-key"));
    let (url, server) = mock(503, json!({"message":"private words"})).await?;
    let error = run(
        &Client::new(),
        "searxng",
        &ProviderConfig {
            base_url: Some(url),
            ..Default::default()
        },
        &request("searxng_search", json!({"query":"private words"})),
    )
    .await
    .err()
    .ok_or("expected provider error")?;
    server.await??;
    assert!(error.to_string().contains("503"));
    assert!(!error.to_string().contains("private words"));
    let invalid = searxng::categories(Some(&json!(["unknown"])));
    assert!(invalid.is_err());
    Ok(())
}

#[test]
fn keyless_searxng_requires_explicit_url_and_direct_route() -> TestResult<()> {
    let mut config = SearchConfig::default();
    config.presentation.mode = crate::PresentationMode::AllTools;
    config
        .providers
        .insert("searxng".into(), ProviderConfig::default());
    let service = SearchService::with_providers(config.clone(), super::super::builtins());
    assert!(
        !service
            .list_tools()
            .tools
            .iter()
            .any(|t| t.name == "searxng_search")
    );
    config
        .providers
        .get_mut("searxng")
        .ok_or("missing searxng provider")?
        .base_url = Some("http://127.0.0.1:8080".into());
    let service = SearchService::with_providers(config.clone(), super::super::builtins());
    assert!(
        service
            .list_tools()
            .tools
            .iter()
            .any(|t| t.name == "searxng_search")
    );
    config
        .providers
        .get_mut("searxng")
        .ok_or("missing searxng provider")?
        .route = ProviderRoute::Backend;
    let service = SearchService::with_providers(config.clone(), super::super::builtins());
    assert!(
        !service
            .list_tools()
            .tools
            .iter()
            .any(|t| t.name == "searxng_search")
    );
    config
        .providers
        .get_mut("searxng")
        .ok_or("missing searxng provider")?
        .route = ProviderRoute::Direct;
    config
        .providers
        .get_mut("searxng")
        .ok_or("missing searxng provider")?
        .enabled = false;
    let service = SearchService::with_providers(config, super::super::builtins());
    assert!(
        !service
            .list_tools()
            .tools
            .iter()
            .any(|t| t.name == "searxng_search")
    );
    Ok(())
}
