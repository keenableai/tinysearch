use super::*;

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn request(name: &str, arguments: Value) -> ExecuteToolRequest {
    ExecuteToolRequest {
        name: name.into(),
        arguments,
    }
}

#[test]
fn rejects_invalid_requests_and_redacts_failed_extraction() -> TestResult<()> {
    for (name, arguments) in [
        ("tavily_search", json!({"query":"  "})),
        ("tavily_extract", json!({"urls":[]})),
    ] {
        assert_eq!(
            prepare(&request(name, arguments), "test-key")
                .err()
                .ok_or("expected provider error")?,
            Error::InvalidArguments
        );
    }
    assert_eq!(
        prepare(&request("unknown", json!({})), "test-key")
            .err()
            .ok_or("expected provider error")?,
        Error::UnavailableTool("unknown".into())
    );
    let extraction = unwrap(
        &request("tavily_extract", json!({})),
        json!({"results":[],"failed_results":[{"url":"https://secret.test","error":"secret body"}]}),
    );
    assert_eq!(
        extraction.err().ok_or("expected provider error")?,
        Error::Provider("Tavily extraction failed for 1 URLs".into())
    );
    let search = unwrap(
        &request("tavily_search", json!({})),
        json!({"answer":"private answer","results":[]}),
    )?;
    assert!(search.get("answer").is_none());
    Ok(())
}
