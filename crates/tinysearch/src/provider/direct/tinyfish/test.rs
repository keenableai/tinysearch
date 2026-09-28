use super::*;

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn request(name: &str, arguments: Value) -> ExecuteToolRequest {
    ExecuteToolRequest {
        name: name.into(),
        arguments,
    }
}

#[test]
fn search_is_a_get_with_the_documented_query_fields() -> TestResult<()> {
    let (method, base, path, body, params, auth, timeout) = prepare(
        &request(
            "tinyfish_search",
            json!({"query":"rust","location":"US","language":"en","page":42,"ignored":true}),
        ),
        "tf-key",
    )?;
    assert_eq!(method, Method::GET);
    assert_eq!(base, SEARCH_BASE);
    assert!(path.is_empty());
    assert!(body.is_none());
    assert_eq!(
        params,
        vec![
            ("query", "rust".to_owned()),
            ("location", "US".to_owned()),
            ("language", "en".to_owned()),
            // Pages are capped at the API's maximum.
            ("page", "10".to_owned()),
        ]
    );
    assert_eq!(auth, ("X-API-Key", "tf-key".to_owned()));
    assert_eq!(timeout, Duration::from_secs(35));
    Ok(())
}

#[test]
fn fetch_posts_urls_and_format_only() -> TestResult<()> {
    let (method, base, path, body, params, _, _) = prepare(
        &request(
            "tinyfish_fetch",
            json!({"urls":["https://a.example","https://b.example"],"format":"html","links":true}),
        ),
        "tf-key",
    )?;
    assert_eq!(method, Method::POST);
    assert_eq!(base, FETCH_BASE);
    assert!(path.is_empty());
    assert!(params.is_empty());
    assert_eq!(
        body,
        Some(json!({"urls":["https://a.example","https://b.example"],"format":"html"}))
    );
    let (.., body, _, _, _) = prepare(
        &request("tinyfish_fetch", json!({"urls":["https://a.example"]})),
        "tf-key",
    )?;
    assert_eq!(body, Some(json!({"urls":["https://a.example"]})));
    Ok(())
}

#[test]
fn agent_run_maps_its_fields_and_proxy_country() -> TestResult<()> {
    let (method, base, path, body, _, _, timeout) = prepare(
        &request(
            "tinyfish_agent_run",
            json!({
                "url":"https://a.example",
                "goal":"find the price",
                "browser_profile":"stealth",
                "use_vault":true,
                "credential_item_ids":["c1"],
                "output_schema":{"type":"object"},
                "proxy_country_code":"DE"
            }),
        ),
        "tf-key",
    )?;
    assert_eq!(method, Method::POST);
    assert_eq!(base, AGENT_BASE);
    assert_eq!(path, "/v1/automation/run");
    assert_eq!(timeout, Duration::from_secs(300));
    assert_eq!(
        body,
        Some(json!({
            "url":"https://a.example",
            "goal":"find the price",
            "browser_profile":"stealth",
            "use_vault":true,
            "credential_item_ids":["c1"],
            "output_schema":{"type":"object"},
            "proxy_config":{"enabled":true,"type":"tetra","country_code":"DE"}
        }))
    );
    Ok(())
}

#[test]
fn rejects_invalid_requests_and_unknown_tools() {
    let eleven: Vec<String> = (0..11).map(|i| format!("https://{i}.example")).collect();
    for (name, arguments, expected) in [
        (
            "tinyfish_search",
            json!({"query":"  "}),
            Error::InvalidArguments,
        ),
        (
            "tinyfish_fetch",
            json!({"urls":[]}),
            Error::InvalidArguments,
        ),
        (
            "tinyfish_fetch",
            json!({"urls":eleven}),
            Error::InvalidArguments,
        ),
        (
            "tinyfish_agent_run",
            json!({"goal":"find"}),
            Error::InvalidArguments,
        ),
        (
            "tinyfish_agent_run",
            json!({"url":"https://a.example"}),
            Error::InvalidArguments,
        ),
        (
            "tinyfish_other",
            json!({}),
            Error::UnavailableTool("tinyfish_other".into()),
        ),
    ] {
        assert_eq!(
            prepare(&request(name, arguments.clone()), "tf-key").err(),
            Some(expected),
            "{name} {arguments}"
        );
    }
}
