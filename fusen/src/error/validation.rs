use std::fmt;

/// Stable classification for configuration validation failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConfigValidationErrorKind {
    /// One value is outside its supported range.
    OutOfRange,
    /// Two or more individually valid values are inconsistent.
    Inconsistent,
}

impl fmt::Display for ConfigValidationErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OutOfRange => "out of range",
            Self::Inconsistent => "inconsistent",
        })
    }
}

/// A safe, field-addressable configuration validation failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
#[error("invalid configuration at {field_path} ({kind}): {reason}")]
pub struct ConfigValidationError {
    kind: ConfigValidationErrorKind,
    field_path: &'static str,
    reason: &'static str,
}

impl ConfigValidationError {
    pub(crate) const fn new(
        kind: ConfigValidationErrorKind,
        field_path: &'static str,
        reason: &'static str,
    ) -> Self {
        Self {
            kind,
            field_path,
            reason,
        }
    }

    /// Returns the stable validation classification.
    pub const fn kind(&self) -> ConfigValidationErrorKind {
        self.kind
    }

    /// Returns the exact public configuration field path.
    pub const fn field_path(&self) -> &'static str {
        self.field_path
    }

    /// Returns a public, credential-free explanation.
    pub const fn reason(&self) -> &'static str {
        self.reason
    }
}
