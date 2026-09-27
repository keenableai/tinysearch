//! Error behavior tests.
use super::Error;
use tinysearch_bus::errors;

#[test]
fn errors_have_actionable_messages() {
    assert!(
        Error::UnavailableTool("missing".into())
            .to_string()
            .contains("missing")
    );
    assert_eq!(
        Error::InvalidArguments.to_string(),
        "tool arguments must be a JSON object"
    );
}

#[test]
fn classified_errors_carry_their_code_on_the_bus() {
    for (error, code) in [
        (Error::InsufficientBalance, errors::INSUFFICIENT_BALANCE),
        (Error::RateLimited, errors::RATE_LIMITED),
        (
            Error::ProviderUnavailable("provider returned HTTP 503".into()),
            errors::UNAVAILABLE,
        ),
        (Error::InvalidArguments, errors::INVALID_ARGUMENTS),
        (
            Error::UnsupportedArgument("extra".into()),
            errors::INVALID_ARGUMENTS,
        ),
        (
            Error::RejectedArguments("provider returned HTTP 422".into()),
            errors::INVALID_ARGUMENTS,
        ),
    ] {
        let message = error.bus_message();
        assert!(
            message.starts_with(&format!("tinysearch.{code}: ")),
            "{message}"
        );
        assert_eq!(errors::code_of(&message), Some(code));
    }
    assert_eq!(Error::Disabled.bus_message(), "search is disabled");
    assert_eq!(Error::Disabled.code(), None);
}

#[test]
fn only_provider_side_failures_are_fallback_eligible() {
    assert!(Error::InsufficientBalance.is_fallback_eligible());
    assert!(Error::RateLimited.is_fallback_eligible());
    assert!(Error::ProviderUnavailable("x".into()).is_fallback_eligible());
    assert!(!Error::InvalidArguments.is_fallback_eligible());
    assert!(!Error::RejectedArguments("x".into()).is_fallback_eligible());
    assert!(!Error::Provider("x".into()).is_fallback_eligible());
}
