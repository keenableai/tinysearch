//! Search validation and routing errors.
use tinysearch_bus::errors;

/// Errors returned by the `TinySearch` service.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Search has been disabled in module configuration.
    #[error("search is disabled")]
    Disabled,
    /// The requested tool is not currently advertised.
    #[error("unknown or unavailable tool: {0}")]
    UnavailableTool(String),
    /// The selected provider is not currently available.
    #[error("unknown or unavailable provider: {0}")]
    UnavailableProvider(String),
    /// Tool arguments must be an object.
    #[error("tool arguments must be a JSON object")]
    InvalidArguments,
    /// An argument is not in the selected tool schema.
    #[error("unsupported tool argument: {0}")]
    UnsupportedArgument(String),
    /// The provider or backend rejected the request as malformed (HTTP 400/422).
    #[error("provider rejected the request arguments: {0}")]
    RejectedArguments(String),
    /// The paying account cannot cover the call (HTTP 402, or a backend
    /// insufficient-credits rejection).
    #[error("insufficient balance for the provider request")]
    InsufficientBalance,
    /// The provider or backend rate limit was reached (HTTP 429).
    #[error("provider rate limit reached")]
    RateLimited,
    /// The provider is unreachable, timed out, or failed on its side.
    #[error("provider unavailable: {0}")]
    ProviderUnavailable(String),
    /// A configured provider is required for this presentation.
    #[error("presentation requires a provider")]
    MissingProvider,
    /// The provider rejected a request.
    #[error("provider request failed: {0}")]
    Provider(String),
}

impl Error {
    /// Returns the stable [`errors`] code for this error, if it has one.
    #[must_use]
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::InvalidArguments | Self::UnsupportedArgument(_) | Self::RejectedArguments(_) => {
                Some(errors::INVALID_ARGUMENTS)
            }
            Self::InsufficientBalance => Some(errors::INSUFFICIENT_BALANCE),
            Self::RateLimited => Some(errors::RATE_LIMITED),
            Self::ProviderUnavailable(_) => Some(errors::UNAVAILABLE),
            _ => None,
        }
    }

    /// Returns whether a role tool should try its next provider after this error.
    #[must_use]
    pub fn is_fallback_eligible(&self) -> bool {
        self.code().is_some_and(errors::is_fallback_eligible)
    }

    /// The message sent over the bus: `tinysearch.<code>: <message>` when the
    /// error has a code, otherwise the plain message.
    #[must_use]
    pub fn bus_message(&self) -> String {
        let message = self.to_string();
        self.code()
            .map_or_else(|| message.clone(), |code| errors::with_code(code, &message))
    }
}

/// Standard result type.
pub type Result<T> = std::result::Result<T, Error>;
#[cfg(test)]
mod test;
