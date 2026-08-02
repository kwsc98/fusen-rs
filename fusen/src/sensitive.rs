//! Safe, policy-driven projections of service invocation values for diagnostics.

use fusen_contract::SensitivityKind;
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeMap, fmt, sync::Arc};

const OMITTED: &str = "<omitted>";

/// The part of a service invocation being projected for diagnostic output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SanitizationTarget {
    /// Named request arguments.
    Arguments,
    /// A successful encoded response body.
    Response,
}

/// Read-only input passed to a [`Sanitizer`] for one classified value.
#[derive(Clone, Copy)]
pub struct SanitizationContext<'a> {
    target: SanitizationTarget,
    path: &'a str,
    kind: SensitivityKind,
    value: &'a Value,
}

impl<'a> SanitizationContext<'a> {
    pub(crate) const fn new(
        target: SanitizationTarget,
        path: &'a str,
        kind: SensitivityKind,
        value: &'a Value,
    ) -> Self {
        Self {
            target,
            path,
            kind,
            value,
        }
    }

    /// Returns whether the value belongs to request arguments or a response.
    pub const fn target(&self) -> SanitizationTarget {
        self.target
    }

    /// Returns the canonical RFC 6901 schema path.
    ///
    /// Array indices are intentionally excluded, so one declared field has one stable path for
    /// every element.
    pub const fn path(&self) -> &str {
        self.path
    }

    /// Returns the sensitivity classification declared by the invocation schema.
    pub const fn kind(&self) -> SensitivityKind {
        self.kind
    }

    /// Returns the original JSON value for policy evaluation.
    pub const fn value(&self) -> &Value {
        self.value
    }
}

impl fmt::Debug for SanitizationContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SanitizationContext")
            .field("target", &self.target)
            .field("path", &self.path)
            .field("kind", &self.kind)
            .field("value", &OMITTED)
            .finish()
    }
}

/// A policy decision for one value with a declared sensitivity kind.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Sanitization {
    /// Leaves the field out of the projected object or array.
    Omit,
    /// Replaces the value with the fixed `<redacted>` marker.
    Redact,
    /// Reveals the value, subject to projection limits.
    Reveal,
    /// Uses a caller-provided safe replacement, subject to projection limits.
    Replace(Value),
}

/// Resource limits applied while building one sanitized projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectionLimits {
    max_input_bytes: usize,
    max_depth: usize,
    max_nodes: usize,
    max_array_items: usize,
    max_string_bytes: usize,
    max_output_bytes: usize,
}

impl ProjectionLimits {
    /// Returns the maximum encoded response bytes accepted for projection.
    pub const fn max_input_bytes(&self) -> usize {
        self.max_input_bytes
    }

    /// Returns the maximum recursive container depth.
    pub const fn max_depth(&self) -> usize {
        self.max_depth
    }

    /// Returns the maximum number of visited JSON nodes.
    pub const fn max_nodes(&self) -> usize {
        self.max_nodes
    }

    /// Returns the maximum number of elements in any projected array.
    pub const fn max_array_items(&self) -> usize {
        self.max_array_items
    }

    /// Returns the maximum UTF-8 byte length of any revealed string.
    pub const fn max_string_bytes(&self) -> usize {
        self.max_string_bytes
    }

    /// Returns the maximum encoded byte length of the complete projection.
    pub const fn max_output_bytes(&self) -> usize {
        self.max_output_bytes
    }

    /// Replaces the encoded response input limit.
    pub const fn with_max_input_bytes(mut self, value: usize) -> Self {
        self.max_input_bytes = value;
        self
    }

    /// Replaces the recursive container depth limit.
    pub const fn with_max_depth(mut self, value: usize) -> Self {
        self.max_depth = value;
        self
    }

    /// Replaces the visited JSON node limit.
    pub const fn with_max_nodes(mut self, value: usize) -> Self {
        self.max_nodes = value;
        self
    }

    /// Replaces the per-array element limit.
    pub const fn with_max_array_items(mut self, value: usize) -> Self {
        self.max_array_items = value;
        self
    }

    /// Replaces the revealed string byte limit.
    pub const fn with_max_string_bytes(mut self, value: usize) -> Self {
        self.max_string_bytes = value;
        self
    }

    /// Replaces the complete encoded output byte limit.
    pub const fn with_max_output_bytes(mut self, value: usize) -> Self {
        self.max_output_bytes = value;
        self
    }
}

impl Default for ProjectionLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 * 1024,
            max_depth: 8,
            max_nodes: 256,
            max_array_items: 32,
            max_string_bytes: 512,
            max_output_bytes: 16 * 1024,
        }
    }
}

/// Object-safe policy for values whose sensitivity is declared by an invocation schema.
pub trait Sanitizer: Send + Sync + 'static {
    /// Decides how one classified value appears in diagnostic output.
    fn sanitize(&self, context: SanitizationContext<'_>) -> Sanitization;

    /// Returns limits for each projection made with this policy.
    fn limits(&self) -> ProjectionLimits {
        ProjectionLimits::default()
    }
}

impl<T> Sanitizer for Arc<T>
where
    T: Sanitizer + ?Sized,
{
    fn sanitize(&self, context: SanitizationContext<'_>) -> Sanitization {
        (**self).sanitize(context)
    }

    fn limits(&self) -> ProjectionLimits {
        (**self).limits()
    }
}

/// A sensitivity-kind policy suitable for application and interceptor logging.
///
/// The default policy reveals values explicitly classified as public, redacts all built-in
/// sensitive kinds, and omits custom kinds until the application adds a rule.
#[derive(Clone, Debug)]
pub struct PolicySanitizer {
    rules: BTreeMap<&'static str, Sanitization>,
    limits: ProjectionLimits,
}

impl PolicySanitizer {
    /// Creates the default fail-closed policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the projection limits.
    pub fn with_limits(mut self, limits: ProjectionLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Adds or replaces the action for one sensitivity kind.
    pub fn with_rule(mut self, kind: SensitivityKind, action: Sanitization) -> Self {
        self.rules.insert(kind.as_str(), action);
        self
    }
}

impl Default for PolicySanitizer {
    fn default() -> Self {
        let mut rules = BTreeMap::new();
        rules.insert(SensitivityKind::PUBLIC.as_str(), Sanitization::Reveal);
        for kind in [
            SensitivityKind::CREDENTIAL,
            SensitivityKind::TOKEN,
            SensitivityKind::PHONE,
            SensitivityKind::EMAIL,
            SensitivityKind::IDENTIFIER,
            SensitivityKind::SECRET,
        ] {
            rules.insert(kind.as_str(), Sanitization::Redact);
        }
        Self {
            rules,
            limits: ProjectionLimits::default(),
        }
    }
}

impl Sanitizer for PolicySanitizer {
    fn sanitize(&self, context: SanitizationContext<'_>) -> Sanitization {
        self.rules
            .get(context.kind().as_str())
            .cloned()
            .unwrap_or(Sanitization::Omit)
    }

    fn limits(&self) -> ProjectionLimits {
        self.limits
    }
}

/// An opaque JSON projection that is safe to pass to diagnostic formatters.
///
/// Omitted projections format as `<omitted>`. Available projections format as JSON and have
/// already passed the active [`ProjectionLimits`].
#[derive(Clone, PartialEq)]
pub struct SanitizedValue {
    value: Option<Value>,
}

impl SanitizedValue {
    pub(crate) const fn omitted() -> Self {
        Self { value: None }
    }

    pub(crate) const fn projected(value: Value) -> Self {
        Self { value: Some(value) }
    }

    /// Returns whether projection failed closed and omitted the complete value.
    pub const fn is_omitted(&self) -> bool {
        self.value.is_none()
    }

    #[cfg(test)]
    pub(crate) const fn as_value(&self) -> Option<&Value> {
        self.value.as_ref()
    }
}

impl fmt::Debug for SanitizedValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for SanitizedValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(value) = &self.value else {
            return formatter.write_str(OMITTED);
        };
        match serde_json::to_string(value) {
            Ok(value) => formatter.write_str(&value),
            Err(_) => formatter.write_str(OMITTED),
        }
    }
}

impl Serialize for SanitizedValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match &self.value {
            Some(value) => value.serialize(serializer),
            None => serializer.serialize_str(OMITTED),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_policy_is_explicit_and_custom_kinds_fail_closed() {
        let policy = PolicySanitizer::default();
        let value = json!("value");
        let decision = |kind| {
            policy.sanitize(SanitizationContext::new(
                SanitizationTarget::Arguments,
                "/value",
                kind,
                &value,
            ))
        };

        assert_eq!(decision(SensitivityKind::PUBLIC), Sanitization::Reveal);
        assert_eq!(decision(SensitivityKind::SECRET), Sanitization::Redact);
        let custom = SensitivityKind::new("application.private").unwrap();
        assert_eq!(decision(custom), Sanitization::Omit);
    }

    #[test]
    fn safe_value_has_consistent_human_and_structured_forms() {
        let value = SanitizedValue::projected(json!({"password": "<redacted>"}));
        assert_eq!(value.to_string(), r#"{"password":"<redacted>"}"#);
        assert_eq!(format!("{value:?}"), value.to_string());
        assert_eq!(
            serde_json::to_value(&value).unwrap(),
            value.as_value().unwrap().clone()
        );

        let omitted = SanitizedValue::omitted();
        assert_eq!(omitted.to_string(), OMITTED);
        assert_eq!(serde_json::to_value(&omitted).unwrap(), json!(OMITTED));
    }

    #[test]
    fn context_debug_never_formats_the_original_value() {
        let value = json!("do-not-log");
        let context = SanitizationContext::new(
            SanitizationTarget::Arguments,
            "/password",
            SensitivityKind::SECRET,
            &value,
        );
        let debug = format!("{context:?}");
        assert!(debug.contains(OMITTED));
        assert!(!debug.contains("do-not-log"));
    }
}
