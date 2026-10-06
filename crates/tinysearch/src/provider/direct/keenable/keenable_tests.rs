use super::*;
use crate::{PresentationMode, SearchConfig, SearchService};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Serves one canned `(status, body)` reply per connection, in order, and
/// returns the raw requests it received.
async fn mock(
    replies: Vec<(u16, Value)>,
) -> TestResult<(String, tokio::task::JoinHandle<TestResult<Vec<String>>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, body) in replies {
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
                    let head = String::from_utf8_lossy(&data[..end + 4]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length: ")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if used >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8_lossy(&data[..used]).into_owned());
            let body = body.to_string();
            let reply = format!(
                "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).await?;
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

fn config(base_url: &str, credential: Option<&str>) -> ProviderConfig {
    ProviderConfig {
        base_url: Some(base_url.into()),
        credential: credential.map(Into::into),
        ..Default::default()
    }
}

fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn body(request: &str) -> TestResult<Value> {
    let (_, body) = request.split_once("\r\n\r\n").ok_or("missing body")?;
    Ok(serde_json::from_str(body)?)
}

#[tokio::test]
async fn keyless_search_uses_the_public_endpoint_and_names_the_caller() -> TestResult<()> {
    let (url, server) = mock(vec![(
        200,
        json!({"query":"rust","results":[
            {"title":"Rust","url":"https://rust.test","description":"","snippet":"Page text","published_at":"2026-01-15T10:30:00Z","acquired_at":"2026-01-16T08:12:34Z"},
            {"title":"Only a description","url":"https://two.test","description":"Summary","snippet":" "},
            {"title":"No URL","snippet":"dropped"}
        ]}),
    )])
    .await?;
    let response = run(
        &Client::new(),
        &config(&url, None),
        &request("keenable_search", json!({"query":"rust"})),
    )
    .await?;
    let sent = server.await??;
    let sent = sent.first().ok_or("no request")?;
    assert!(sent.starts_with("POST /search/public "));
    assert_eq!(header(sent, "x-keenable-title"), Some("tinysearch"));
    assert_eq!(header(sent, "x-api-key"), None);
    assert_eq!(
        body(sent)?,
        json!({"query":"rust","max_results":5,"snippet_max_length":1200})
    );
    assert_eq!(response.provider, "keenable");
    assert_eq!(response.results.len(), 2);
    assert_eq!(response.results[0].snippet.as_deref(), Some("Page text"));
    assert_eq!(
        response.results[0].published.as_deref(),
        Some("2026-01-15T10:30:00Z")
    );
    assert_eq!(response.results[1].snippet.as_deref(), Some("Summary"));
    assert_eq!(response.results[1].published, None);
    Ok(())
}

#[tokio::test]
async fn keyed_search_uses_the_key_the_configured_count_and_filters() -> TestResult<()> {
    let (url, server) = mock(vec![(200, json!({"results":[]}))]).await?;
    let mut config = config(&url, Some(" keen_test "));
    config.max_results = Some(50);
    let response = run(
        &Client::new(),
        &config,
        &request(
            "keenable_search",
            json!({"query":"rust","site":"rust-lang.org","published_after":"7d","published_before":"2026-10-01","ignored":true}),
        ),
    )
    .await?;
    let sent = server.await??;
    let sent = sent.first().ok_or("no request")?;
    assert!(sent.starts_with("POST /search "));
    assert_eq!(header(sent, "x-api-key"), Some("keen_test"));
    assert_eq!(header(sent, "x-keenable-title"), Some("tinysearch"));
    assert_eq!(
        body(sent)?,
        json!({"query":"rust","max_results":20,"snippet_max_length":1200,"site":"rust-lang.org","published_after":"7d","published_before":"2026-10-01"})
    );
    assert_eq!(response.status, crate::SearchStatus::Empty);
    Ok(())
}

#[tokio::test]
async fn fetch_reads_each_url_and_retries_an_unindexed_page_live() -> TestResult<()> {
    let (url, server) = mock(vec![
        (
            200,
            json!({"url":"https://a.test/","title":"A","description":"","content":"# A"}),
        ),
        (404, json!({"error":"Not found","message":"private"})),
        (
            200,
            json!({"url":"https://b.test/","title":"B","content":"# B"}),
        ),
    ])
    .await?;
    let response = run(
        &Client::new(),
        &config(&url, None),
        &request(
            "keenable_fetch",
            json!({"urls":["https://a.test/","https://b.test/"]}),
        ),
    )
    .await?;
    let sent = server.await??;
    assert_eq!(sent.len(), 3);
    assert!(sent[0].starts_with("GET /fetch/public?url=https%3A%2F%2Fa.test%2F "));
    assert!(sent[1].starts_with("GET /fetch/public?url=https%3A%2F%2Fb.test%2F "));
    assert!(sent[2].starts_with("GET /fetch/public?url=https%3A%2F%2Fb.test%2F&live=true "));
    assert!(
        sent.iter()
            .all(|s| header(s, "x-keenable-title") == Some("tinysearch"))
    );
    assert_eq!(response.results.len(), 2);
    assert_eq!(response.results[0].title, "A");
    assert_eq!(response.results[0].snippet.as_deref(), Some("# A"));
    assert_eq!(response.results[1].url, "https://b.test/");
    assert_eq!(
        response.provider_data.ok_or("missing provider data")?["failed_count"],
        0
    );
    Ok(())
}

#[tokio::test]
async fn fetch_with_a_key_uses_the_keyed_endpoint_and_reports_partial_failures() -> TestResult<()> {
    let (url, server) = mock(vec![
        (503, json!({"message":"busy"})),
        (200, json!({"content":"page text"})),
    ])
    .await?;
    let response = run(
        &Client::new(),
        &config(&url, Some("keen_test")),
        &request(
            "keenable_fetch",
            json!({"urls":["https://a.test/","https://b.test/"]}),
        ),
    )
    .await?;
    let sent = server.await??;
    assert!(sent[0].starts_with("GET /fetch?url="));
    assert_eq!(header(&sent[1], "x-api-key"), Some("keen_test"));
    assert_eq!(response.results.len(), 1);
    // A page without its own URL or title keeps the requested URL.
    assert_eq!(response.results[0].url, "https://b.test/");
    assert_eq!(response.results[0].title, "");
    assert_eq!(
        response.provider_data.ok_or("missing provider data")?["failed_count"],
        1
    );
    Ok(())
}

#[tokio::test]
async fn a_fetch_where_every_page_fails_keeps_the_classification() -> TestResult<()> {
    let (url, server) = mock(vec![(
        429,
        json!({"message":"Public API hourly limit reached"}),
    )])
    .await?;
    let error = run(
        &Client::new(),
        &config(&url, None),
        &request("keenable_fetch", json!({"urls":["https://a.test/"]})),
    )
    .await
    .err()
    .ok_or("expected an error")?;
    server.await??;
    // Rate limits stay fallback-eligible so the contents role can try the next provider.
    assert!(matches!(error, Error::RateLimited));
    Ok(())
}

#[tokio::test]
async fn invalid_requests_fail_before_any_call() -> TestResult<()> {
    let config = config("http://127.0.0.1:9", None);
    let client = Client::new();
    let unknown = run(&client, &config, &request("keenable_answer", json!({}))).await;
    assert!(matches!(unknown, Err(Error::UnavailableTool(name)) if name == "keenable_answer"));
    let no_query = run(
        &client,
        &config,
        &request("keenable_search", json!({"query":" "})),
    )
    .await;
    assert!(matches!(no_query, Err(Error::InvalidArguments)));
    let no_urls = run(
        &client,
        &config,
        &request("keenable_fetch", json!({"urls":[]})),
    )
    .await;
    assert!(matches!(no_urls, Err(Error::InvalidArguments)));
    let bad_base = run(
        &client,
        &ProviderConfig {
            base_url: Some("ftp://keenable.test".into()),
            ..Default::default()
        },
        &request("keenable_search", json!({"query":"rust"})),
    )
    .await;
    assert!(matches!(bad_base, Err(Error::Provider(_))));
    Ok(())
}

#[test]
fn keenable_is_listed_without_a_credential_only_when_enabled_and_direct() -> TestResult<()> {
    let tool_names = |config: &SearchConfig| -> Vec<String> {
        SearchService::with_providers(config.clone(), super::super::super::builtins())
            .list_tools()
            .tools
            .into_iter()
            .map(|tool| tool.name)
            .collect()
    };
    let mut config = SearchConfig::default();
    config.presentation.mode = PresentationMode::AllTools;
    assert_eq!(tool_names(&config).len(), 0);
    config
        .providers
        .insert("keenable".into(), ProviderConfig::default());
    assert_eq!(tool_names(&config), ["keenable_search", "keenable_fetch"]);
    config.presentation.mode = PresentationMode::Roles;
    assert_eq!(
        tool_names(&config),
        ["web_search_tool", "web_contents_tool"]
    );
    let keenable = config
        .providers
        .get_mut("keenable")
        .ok_or("missing keenable provider")?;
    keenable.route = crate::ProviderRoute::Backend;
    assert_eq!(tool_names(&config).len(), 0);
    let keenable = config
        .providers
        .get_mut("keenable")
        .ok_or("missing keenable provider")?;
    keenable.route = crate::ProviderRoute::Direct;
    keenable.enabled = false;
    assert_eq!(tool_names(&config).len(), 0);
    Ok(())
}
