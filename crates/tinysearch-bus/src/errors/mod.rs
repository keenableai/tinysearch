//! Stable error codes carried in `TinySearch` bus error messages.
//!
//! A failed `ExecuteTool` call reaches the host as a `TinyBus` method error
//! whose message starts with `tinysearch.<code>: ` when the module can
//! classify the failure. Hosts classify with [`code_of`] instead of matching
//! prose. Messages without a code are unclassified failures.

/// Prefix that introduces a code in a bus error message.
pub const PREFIX: &str = "tinysearch.";

/// The account paying for the call (the managed backend balance or the
/// provider's own plan) cannot cover it. Role tools fall back past it.
pub const INSUFFICIENT_BALANCE: &str = "insufficient_balance";
/// The provider or backend rate limit was reached. Role tools fall back past it.
pub const RATE_LIMITED: &str = "rate_limited";
/// The provider is unreachable, timed out, or failed on its side (HTTP 5xx).
/// Role tools fall back past it.
pub const UNAVAILABLE: &str = "provider_unavailable";
/// The request arguments were rejected. Role tools do not fall back past it,
/// because another provider would reject the same request.
pub const INVALID_ARGUMENTS: &str = "invalid_arguments";

/// Every code, for exhaustive host-side matching.
pub const ALL: &[&str] = &[INSUFFICIENT_BALANCE, RATE_LIMITED, UNAVAILABLE, INVALID_ARGUMENTS];

/// Returns whether a role tool tries the next provider after `code`.
#[must_use]
pub fn is_fallback_eligible(code: &str) -> bool {
    matches!(code, INSUFFICIENT_BALANCE | RATE_LIMITED | UNAVAILABLE)
}

/// Formats `message` with the `tinysearch.<code>: ` prefix.
#[must_use]
pub fn with_code(code: &str, message: &str) -> String {
    format!("{PREFIX}{code}: {message}")
}

/// Extracts the code from a bus error message.
///
/// The code may appear after a transport-added prefix, so the first
/// `tinysearch.<code>: ` occurrence anywhere in the message counts.
///
/// # Examples
///
/// ```
/// # use tinysearch_bus::errors;
/// assert_eq!(
///     errors::code_of("tinysearch.rate_limited: provider rate limit reached"),
///     Some(errors::RATE_LIMITED)
/// );
/// assert_eq!(errors::code_of("search is disabled"), None);
/// ```
#[must_use]
pub fn code_of(message: &str) -> Option<&'static str> {
    message.match_indices(PREFIX).find_map(|(index, _)| {
        let rest = &message[index + PREFIX.len()..];
        ALL.iter()
            .copied()
            .find(|code| rest.strip_prefix(code).is_some_and(|tail| tail.starts_with(": ")))
    })
}

#[cfg(test)]
mod test;
