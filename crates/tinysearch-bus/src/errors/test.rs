//! Error code wire tests.
use super::*;

#[test]
fn codes_are_stable_wire_strings() {
    assert_eq!(INSUFFICIENT_BALANCE, "insufficient_balance");
    assert_eq!(RATE_LIMITED, "rate_limited");
    assert_eq!(UNAVAILABLE, "provider_unavailable");
    assert_eq!(INVALID_ARGUMENTS, "invalid_arguments");
}

#[test]
fn prefixed_messages_round_trip_through_code_of() {
    for code in ALL {
        let message = with_code(code, "details");
        assert!(message.starts_with(&format!("tinysearch.{code}: ")));
        assert_eq!(code_of(&message), Some(*code));
        assert_eq!(code_of(&format!("method failed: {message}")), Some(*code));
    }
    assert_eq!(code_of("tinysearch.unknown: nope"), None);
    assert_eq!(code_of("tinysearch.rate_limitedx: nope"), None);
    assert_eq!(code_of("plain failure"), None);
}

#[test]
fn only_provider_side_conditions_fall_back() {
    assert!(is_fallback_eligible(INSUFFICIENT_BALANCE));
    assert!(is_fallback_eligible(RATE_LIMITED));
    assert!(is_fallback_eligible(UNAVAILABLE));
    assert!(!is_fallback_eligible(INVALID_ARGUMENTS));
}
