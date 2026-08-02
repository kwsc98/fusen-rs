use crate::{
    Arguments,
    sensitive::{
        ProjectionLimits, Sanitization, SanitizationContext, SanitizationTarget, SanitizedValue,
        Sanitizer,
    },
};
use fusen_contract::{MethodDescriptor, SensitiveShape};
use serde_json::{Map, Value};
use std::{
    collections::BTreeSet,
    panic::{AssertUnwindSafe, catch_unwind},
};

const REDACTED: &str = "<redacted>";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProjectionDirection {
    Serialize,
    Deserialize,
}

impl ProjectionDirection {
    const fn fields<'a>(
        self,
        serialize: &'a [fusen_contract::SensitiveField],
        deserialize: &'a [fusen_contract::SensitiveField],
    ) -> &'a [fusen_contract::SensitiveField] {
        match self {
            Self::Serialize => serialize,
            Self::Deserialize => deserialize,
        }
    }
}

pub(crate) fn sanitize_arguments(
    method: &MethodDescriptor,
    arguments: &Arguments,
    direction: ProjectionDirection,
    sanitizer: &dyn Sanitizer,
) -> SanitizedValue {
    fail_closed(|| {
        let sensitivity = method.sensitivity().ok_or(ProjectionFailure)?;
        let mut projector = Projector::new(sanitizer, SanitizationTarget::Arguments, direction);
        let mut projected = Map::new();
        let mut declared = BTreeSet::new();

        for argument in sensitivity.arguments() {
            if !declared.insert(argument.name()) {
                return Err(ProjectionFailure);
            }
            let Some(value) = arguments.get(argument.name()) else {
                continue;
            };
            let mut path = String::new();
            push_path_segment(&mut path, argument.name());
            if let Projected::Value(value) =
                projector.project(value, argument.shape(), 0, &mut path)?
            {
                projected.insert(argument.name().to_owned(), value);
            }
        }

        if projected.is_empty() {
            projector.finish(Projected::Omit)
        } else {
            projector.finish(Projected::Value(Value::Object(projected)))
        }
    })
}

pub(crate) fn sanitize_response(
    method: &MethodDescriptor,
    bytes: &[u8],
    direction: Option<ProjectionDirection>,
    sanitizer: &dyn Sanitizer,
) -> SanitizedValue {
    let Some(direction) = direction else {
        return SanitizedValue::omitted();
    };
    fail_closed(|| {
        let shape = method
            .sensitivity()
            .and_then(|sensitivity| sensitivity.response_shape())
            .ok_or(ProjectionFailure)?;
        if matches!(shape, SensitiveShape::Opaque) {
            return Ok(SanitizedValue::omitted());
        }
        let mut projector = Projector::new(sanitizer, SanitizationTarget::Response, direction);
        if bytes.len() > projector.limits.max_input_bytes() {
            return Err(ProjectionFailure);
        }
        let value = serde_json::from_slice(bytes).map_err(|_| ProjectionFailure)?;
        let projected = projector.project(&value, shape, 0, &mut String::new())?;
        projector.finish(projected)
    })
}

fn fail_closed(
    project: impl FnOnce() -> Result<SanitizedValue, ProjectionFailure>,
) -> SanitizedValue {
    match catch_unwind(AssertUnwindSafe(project)) {
        Ok(Ok(value)) => value,
        Ok(Err(ProjectionFailure)) | Err(_) => SanitizedValue::omitted(),
    }
}

struct Projector<'a> {
    sanitizer: &'a dyn Sanitizer,
    target: SanitizationTarget,
    direction: ProjectionDirection,
    limits: ProjectionLimits,
    visited_nodes: usize,
}

impl<'a> Projector<'a> {
    fn new(
        sanitizer: &'a dyn Sanitizer,
        target: SanitizationTarget,
        direction: ProjectionDirection,
    ) -> Self {
        Self {
            sanitizer,
            target,
            direction,
            limits: sanitizer.limits(),
            visited_nodes: 0,
        }
    }

    fn finish(self, projected: Projected) -> Result<SanitizedValue, ProjectionFailure> {
        let Projected::Value(value) = projected else {
            return Ok(SanitizedValue::omitted());
        };
        let encoded = serde_json::to_vec(&value).map_err(|_| ProjectionFailure)?;
        if encoded.len() > self.limits.max_output_bytes() {
            return Err(ProjectionFailure);
        }
        Ok(SanitizedValue::projected(value))
    }

    fn project(
        &mut self,
        value: &Value,
        shape: SensitiveShape,
        depth: usize,
        path: &mut String,
    ) -> Result<Projected, ProjectionFailure> {
        self.visit(depth)?;
        self.project_visited(value, shape, depth, path)
    }

    fn project_visited(
        &mut self,
        value: &Value,
        shape: SensitiveShape,
        depth: usize,
        path: &mut String,
    ) -> Result<Projected, ProjectionFailure> {
        if matches!(shape, SensitiveShape::Opaque) {
            return Ok(Projected::Omit);
        }
        match shape {
            SensitiveShape::Opaque => unreachable!("opaque shapes return before projection"),
            SensitiveShape::Kind(kind) => {
                self.validate_descendants(value, depth)?;
                self.sanitize_classified(value, kind, depth, path)
            }
            SensitiveShape::Fields {
                serialize,
                deserialize,
            } => {
                let fields = self.direction.fields(serialize, deserialize);
                let Value::Object(values) = value else {
                    return Err(ProjectionFailure);
                };
                let mut projected = Map::new();
                let mut declared = BTreeSet::new();
                for field in fields {
                    if !declared.insert(field.name()) {
                        return Err(ProjectionFailure);
                    }
                    let Some(value) = values.get(field.name()) else {
                        continue;
                    };
                    let previous_length = path.len();
                    push_path_segment(path, field.name());
                    let result = self.project(value, field.shape(), depth + 1, path);
                    path.truncate(previous_length);
                    if let Projected::Value(value) = result? {
                        projected.insert(field.name().to_owned(), value);
                    }
                }
                if projected.is_empty() {
                    Ok(Projected::Omit)
                } else {
                    Ok(Projected::Value(Value::Object(projected)))
                }
            }
            SensitiveShape::Optional(resolver) => {
                let inner = resolver();
                match self.inherited_classification(inner, 0)? {
                    Some(InheritedClassification::Opaque) => Ok(Projected::Omit),
                    Some(InheritedClassification::Kind(kind)) => {
                        self.validate_shape_visited(
                            value,
                            SensitiveShape::Optional(resolver),
                            depth,
                        )?;
                        self.sanitize_classified(value, kind, depth, path)
                    }
                    None if value.is_null() => Ok(Projected::Omit),
                    None => self.project_visited(value, inner, depth, path),
                }
            }
            SensitiveShape::Sequence(resolver) => {
                self.project_sequence(value, resolver, None, depth, path)
            }
            SensitiveShape::FixedSequence { element, length } => {
                self.project_sequence(value, element, Some(length), depth, path)
            }
            _ => Err(ProjectionFailure),
        }
    }

    fn project_sequence(
        &mut self,
        value: &Value,
        resolver: fusen_contract::SensitiveShapeResolver,
        required_length: Option<usize>,
        depth: usize,
        path: &mut String,
    ) -> Result<Projected, ProjectionFailure> {
        let Value::Array(values) = value else {
            return Err(ProjectionFailure);
        };
        if values.len() > self.limits.max_array_items()
            || required_length.is_some_and(|length| values.len() != length)
        {
            return Err(ProjectionFailure);
        }
        let element = resolver();
        match self.inherited_classification(element, 0)? {
            Some(InheritedClassification::Opaque) => Ok(Projected::Omit),
            Some(InheritedClassification::Kind(kind)) => {
                for value in values {
                    self.validate_shape(value, element, depth + 1)?;
                }
                self.sanitize_classified(value, kind, depth, path)
            }
            None => {
                let mut projected = Vec::with_capacity(values.len());
                for value in values {
                    if let Projected::Value(value) =
                        self.project(value, element, depth + 1, path)?
                    {
                        projected.push(value);
                    }
                }
                if projected.is_empty()
                    && (!values.is_empty() || !self.has_classified_value(element, 0)?)
                {
                    Ok(Projected::Omit)
                } else {
                    Ok(Projected::Value(Value::Array(projected)))
                }
            }
        }
    }

    fn sanitize_classified(
        &mut self,
        value: &Value,
        kind: fusen_contract::SensitivityKind,
        depth: usize,
        path: &str,
    ) -> Result<Projected, ProjectionFailure> {
        let context = SanitizationContext::new(self.target, path, kind, value);
        match self.sanitizer.sanitize(context) {
            Sanitization::Omit => Ok(Projected::Omit),
            Sanitization::Redact => Ok(Projected::Value(Value::String(REDACTED.to_owned()))),
            Sanitization::Reveal => Ok(Projected::Value(value.clone())),
            Sanitization::Replace(replacement) => {
                self.validate_value(&replacement, depth)?;
                Ok(Projected::Value(replacement))
            }
        }
    }

    fn inherited_classification(
        &self,
        shape: SensitiveShape,
        schema_depth: usize,
    ) -> Result<Option<InheritedClassification>, ProjectionFailure> {
        if schema_depth > self.limits.max_depth() {
            return Err(ProjectionFailure);
        }
        match shape {
            SensitiveShape::Opaque => Ok(Some(InheritedClassification::Opaque)),
            SensitiveShape::Kind(kind) => Ok(Some(InheritedClassification::Kind(kind))),
            SensitiveShape::Fields { .. } => Ok(None),
            SensitiveShape::Optional(resolver) | SensitiveShape::Sequence(resolver) => {
                self.inherited_classification(resolver(), schema_depth + 1)
            }
            SensitiveShape::FixedSequence { element, .. } => {
                self.inherited_classification(element(), schema_depth + 1)
            }
            _ => Err(ProjectionFailure),
        }
    }

    fn has_classified_value(
        &self,
        shape: SensitiveShape,
        schema_depth: usize,
    ) -> Result<bool, ProjectionFailure> {
        if schema_depth > self.limits.max_depth() {
            return Err(ProjectionFailure);
        }
        match shape {
            SensitiveShape::Opaque => Ok(false),
            SensitiveShape::Kind(_) => Ok(true),
            SensitiveShape::Fields {
                serialize,
                deserialize,
            } => {
                let fields = self.direction.fields(serialize, deserialize);
                for field in fields {
                    if self.has_classified_value(field.shape(), schema_depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            SensitiveShape::Optional(resolver) | SensitiveShape::Sequence(resolver) => {
                self.has_classified_value(resolver(), schema_depth + 1)
            }
            SensitiveShape::FixedSequence { element, .. } => {
                self.has_classified_value(element(), schema_depth + 1)
            }
            _ => Err(ProjectionFailure),
        }
    }

    fn validate_shape(
        &mut self,
        value: &Value,
        shape: SensitiveShape,
        depth: usize,
    ) -> Result<(), ProjectionFailure> {
        self.visit(depth)?;
        self.validate_shape_visited(value, shape, depth)
    }

    fn validate_shape_visited(
        &mut self,
        value: &Value,
        shape: SensitiveShape,
        depth: usize,
    ) -> Result<(), ProjectionFailure> {
        match shape {
            SensitiveShape::Opaque => Ok(()),
            SensitiveShape::Kind(_) => self.validate_descendants(value, depth),
            SensitiveShape::Optional(_) if value.is_null() => Ok(()),
            SensitiveShape::Optional(resolver) => {
                self.validate_shape_visited(value, resolver(), depth)
            }
            SensitiveShape::Sequence(resolver) => {
                self.validate_sequence(value, resolver, None, depth)
            }
            SensitiveShape::FixedSequence { element, length } => {
                self.validate_sequence(value, element, Some(length), depth)
            }
            SensitiveShape::Fields { .. } => {
                if value.is_object() {
                    Ok(())
                } else {
                    Err(ProjectionFailure)
                }
            }
            _ => Err(ProjectionFailure),
        }
    }

    fn validate_sequence(
        &mut self,
        value: &Value,
        resolver: fusen_contract::SensitiveShapeResolver,
        required_length: Option<usize>,
        depth: usize,
    ) -> Result<(), ProjectionFailure> {
        let Value::Array(values) = value else {
            return Err(ProjectionFailure);
        };
        if values.len() > self.limits.max_array_items()
            || required_length.is_some_and(|length| values.len() != length)
        {
            return Err(ProjectionFailure);
        }
        let element = resolver();
        for value in values {
            self.validate_shape(value, element, depth + 1)?;
        }
        Ok(())
    }

    fn validate_value(&mut self, value: &Value, depth: usize) -> Result<(), ProjectionFailure> {
        self.visit(depth)?;
        self.validate_descendants(value, depth)
    }

    fn validate_descendants(
        &mut self,
        value: &Value,
        depth: usize,
    ) -> Result<(), ProjectionFailure> {
        match value {
            Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
            Value::String(value) => {
                if value.len() > self.limits.max_string_bytes() {
                    return Err(ProjectionFailure);
                }
                Ok(())
            }
            Value::Array(values) => {
                if values.len() > self.limits.max_array_items() {
                    return Err(ProjectionFailure);
                }
                for value in values {
                    self.validate_value(value, depth + 1)?;
                }
                Ok(())
            }
            Value::Object(values) => {
                for (name, value) in values {
                    if name.len() > self.limits.max_string_bytes() {
                        return Err(ProjectionFailure);
                    }
                    self.validate_value(value, depth + 1)?;
                }
                Ok(())
            }
        }
    }

    fn visit(&mut self, depth: usize) -> Result<(), ProjectionFailure> {
        if depth > self.limits.max_depth() {
            return Err(ProjectionFailure);
        }
        self.visited_nodes = self.visited_nodes.checked_add(1).ok_or(ProjectionFailure)?;
        if self.visited_nodes > self.limits.max_nodes() {
            return Err(ProjectionFailure);
        }
        Ok(())
    }
}

enum Projected {
    Omit,
    Value(Value),
}

#[derive(Clone, Copy)]
enum InheritedClassification {
    Opaque,
    Kind(fusen_contract::SensitivityKind),
}

#[derive(Clone, Copy)]
struct ProjectionFailure;

fn push_path_segment(path: &mut String, segment: &str) {
    path.push('/');
    for character in segment.chars() {
        match character {
            '~' => path.push_str("~0"),
            '/' => path.push_str("~1"),
            character => path.push(character),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensitive::{PolicySanitizer, ProjectionLimits};
    use fusen_contract::{
        HttpOperation, MethodId, MethodSensitivity, SensitiveArgument, SensitiveField,
        SensitivityKind,
    };
    use serde_json::json;
    use std::sync::Mutex;

    fn http_operation() -> HttpOperation {
        HttpOperation::new(
            http::Method::POST,
            "/test",
            Vec::new(),
            "application/json",
            "application/json",
        )
        .unwrap()
    }

    fn public() -> SensitiveShape {
        SensitiveShape::Kind(SensitivityKind::PUBLIC)
    }

    fn secret() -> SensitiveShape {
        SensitiveShape::Kind(SensitivityKind::SECRET)
    }

    fn optional_secret() -> SensitiveShape {
        SensitiveShape::Optional(secret)
    }

    fn secret_sequence() -> SensitiveShape {
        SensitiveShape::Sequence(secret)
    }

    fn opaque() -> SensitiveShape {
        SensitiveShape::Opaque
    }

    fn panic_shape() -> SensitiveShape {
        panic!("schema resolver panic must not escape")
    }

    fn symmetric_fields(fields: &'static [SensitiveField]) -> SensitiveShape {
        SensitiveShape::Fields {
            serialize: fields,
            deserialize: fields,
        }
    }

    fn child() -> SensitiveShape {
        symmetric_fields(&[
            const { SensitiveField::new("name", public) },
            const { SensitiveField::new("secret", secret) },
        ])
    }

    fn children() -> SensitiveShape {
        SensitiveShape::Sequence(child)
    }

    fn optional_child() -> SensitiveShape {
        SensitiveShape::Optional(child)
    }

    fn optional_children() -> SensitiveShape {
        SensitiveShape::Optional(children)
    }

    fn fixed_secrets() -> SensitiveShape {
        SensitiveShape::FixedSequence {
            element: secret,
            length: 2,
        }
    }

    fn oversized_fixed_secrets() -> SensitiveShape {
        SensitiveShape::FixedSequence {
            element: secret,
            length: 33,
        }
    }

    fn all_opaque() -> SensitiveShape {
        symmetric_fields(&[const { SensitiveField::new("value", opaque) }])
    }

    fn all_opaque_sequence() -> SensitiveShape {
        SensitiveShape::Sequence(all_opaque)
    }

    fn request() -> SensitiveShape {
        symmetric_fields(&[
            const { SensitiveField::new("id", public) },
            const { SensitiveField::new("password", secret) },
            const { SensitiveField::new("children", children) },
            const { SensitiveField::new("opaque", opaque) },
            const { SensitiveField::new("opaque_null", opaque) },
        ])
    }

    fn response_shape() -> SensitiveShape {
        child()
    }

    fn directional_payload() -> SensitiveShape {
        SensitiveShape::Fields {
            serialize: &[
                const { SensitiveField::new("display", public) },
                const { SensitiveField::new("password", secret) },
            ],
            deserialize: &[
                const { SensitiveField::new("public_input", public) },
                const { SensitiveField::new("display", secret) },
            ],
        }
    }

    fn method(with_response: bool) -> MethodDescriptor {
        MethodDescriptor::new(MethodId::new(0), "project", http_operation())
            .unwrap()
            .with_sensitivity(MethodSensitivity::new(
                vec![SensitiveArgument::new("request", request)],
                with_response.then_some(response_shape),
            ))
    }

    fn invocation_arguments(value: Value) -> Arguments {
        let Value::Object(fields) = value else {
            panic!("test arguments must be an object")
        };
        let mut arguments = Arguments::new();
        arguments.extend(fields);
        arguments
    }

    fn sanitize_serialized_arguments(
        method: &MethodDescriptor,
        arguments: &Arguments,
        sanitizer: &dyn Sanitizer,
    ) -> SanitizedValue {
        sanitize_arguments(method, arguments, ProjectionDirection::Serialize, sanitizer)
    }

    #[test]
    fn recursive_projection_whitelists_fields_and_arrays() {
        let arguments = invocation_arguments(json!({
            "request": {
                "id": 7,
                "password": "do-not-log",
                "unknown": "also-do-not-log",
                "opaque": "not-classified",
                "opaque_null": null,
                "children": [
                    {"name": "first", "secret": "one", "unknown": true},
                    {"name": "second", "secret": "two"}
                ]
            },
            "undeclared": "hidden"
        }));
        let projected =
            sanitize_serialized_arguments(&method(false), &arguments, &PolicySanitizer::default());

        assert_eq!(
            projected.as_value(),
            Some(&json!({
                "request": {
                    "id": 7,
                    "password": "<redacted>",
                    "children": [
                        {"name": "first", "secret": "<redacted>"},
                        {"name": "second", "secret": "<redacted>"}
                    ]
                }
            }))
        );
        assert!(!projected.to_string().contains("do-not-log"));
        assert!(!projected.to_string().contains("unknown"));
    }

    #[test]
    fn field_tables_are_selected_by_representation_direction() {
        let descriptor = MethodDescriptor::new(MethodId::new(0), "directional", http_operation())
            .unwrap()
            .with_sensitivity(MethodSensitivity::new(
                vec![SensitiveArgument::new("request", directional_payload)],
                Some(directional_payload),
            ));
        let policy = PolicySanitizer::default();

        let serialized = invocation_arguments(json!({
            "request": {"display": "safe", "password": "outbound-secret"}
        }));
        assert_eq!(
            sanitize_arguments(
                &descriptor,
                &serialized,
                ProjectionDirection::Serialize,
                &policy,
            )
            .as_value(),
            Some(&json!({
                "request": {"display": "safe", "password": "<redacted>"}
            }))
        );

        let deserialized = invocation_arguments(json!({
            "request": {"public_input": "safe", "display": "inbound-secret"}
        }));
        assert_eq!(
            sanitize_arguments(
                &descriptor,
                &deserialized,
                ProjectionDirection::Deserialize,
                &policy,
            )
            .as_value(),
            Some(&json!({
                "request": {"public_input": "safe", "display": "<redacted>"}
            }))
        );

        assert_eq!(
            sanitize_response(
                &descriptor,
                br#"{"display":"safe","password":"response-secret"}"#,
                Some(ProjectionDirection::Serialize),
                &policy,
            )
            .as_value(),
            Some(&json!({"display": "safe", "password": "<redacted>"}))
        );
        assert_eq!(
            sanitize_response(
                &descriptor,
                br#"{"public_input":"safe","display":"response-secret"}"#,
                Some(ProjectionDirection::Deserialize),
                &policy,
            )
            .as_value(),
            Some(&json!({"public_input": "safe", "display": "<redacted>"}))
        );
    }

    #[test]
    fn fully_unclassified_objects_and_nonempty_arrays_are_omitted() {
        let descriptor = MethodDescriptor::new(MethodId::new(0), "opaque", http_operation())
            .unwrap()
            .with_sensitivity(MethodSensitivity::new(
                vec![
                    SensitiveArgument::new("object", all_opaque),
                    SensitiveArgument::new("objects", all_opaque_sequence),
                    SensitiveArgument::new("empty_objects", all_opaque_sequence),
                ],
                Some(all_opaque),
            ));
        let arguments = invocation_arguments(json!({
            "object": {"value": "hidden"},
            "objects": [{"value": "one"}, {"value": "two"}],
            "empty_objects": []
        }));
        let policy = PolicySanitizer::default();

        assert!(sanitize_serialized_arguments(&descriptor, &arguments, &policy).is_omitted());
        assert!(
            sanitize_response(
                &descriptor,
                br#"{"value":"hidden"}"#,
                Some(ProjectionDirection::Serialize),
                &policy,
            )
            .is_omitted()
        );
    }

    #[derive(Default)]
    struct RecordingSanitizer {
        paths: Mutex<Vec<String>>,
        limits: Option<ProjectionLimits>,
    }

    impl Sanitizer for RecordingSanitizer {
        fn sanitize(&self, context: SanitizationContext<'_>) -> Sanitization {
            self.paths.lock().unwrap().push(context.path().to_owned());
            if context.kind() == SensitivityKind::PUBLIC {
                Sanitization::Reveal
            } else {
                Sanitization::Redact
            }
        }

        fn limits(&self) -> ProjectionLimits {
            self.limits.unwrap_or_default()
        }
    }

    fn escaped_child() -> SensitiveShape {
        symmetric_fields(&[const { SensitiveField::new("~token", secret) }])
    }

    fn escaped_children() -> SensitiveShape {
        SensitiveShape::Sequence(escaped_child)
    }

    fn escaped_request() -> SensitiveShape {
        symmetric_fields(&[const { SensitiveField::new("a/b", escaped_children) }])
    }

    #[test]
    fn paths_are_canonical_and_do_not_include_array_indices() {
        let method = MethodDescriptor::new(MethodId::new(0), "escaped", http_operation())
            .unwrap()
            .with_sensitivity(MethodSensitivity::new(
                vec![SensitiveArgument::new("request", escaped_request)],
                None,
            ));
        let arguments = invocation_arguments(json!({
            "request": {"a/b": [{"~token": "a"}, {"~token": "b"}]}
        }));
        let sanitizer = RecordingSanitizer::default();

        let projected = sanitize_serialized_arguments(&method, &arguments, &sanitizer);

        assert!(!projected.is_omitted());
        assert_eq!(
            *sanitizer.paths.lock().unwrap(),
            ["/request/a~1b/~0token", "/request/a~1b/~0token"]
        );
    }

    #[test]
    fn kind_classifies_complete_arrays_and_nulls() {
        let method = MethodDescriptor::new(MethodId::new(0), "complete-values", http_operation())
            .unwrap()
            .with_sensitivity(MethodSensitivity::new(
                vec![
                    SensitiveArgument::new("tokens", secret_sequence),
                    SensitiveArgument::new("nullable", optional_secret),
                ],
                None,
            ));
        let arguments = invocation_arguments(json!({
            "tokens": ["first", "second"],
            "nullable": null
        }));
        let sanitizer = RecordingSanitizer::default();

        let projected = sanitize_serialized_arguments(&method, &arguments, &sanitizer);

        assert_eq!(
            projected.as_value(),
            Some(&json!({
                "tokens": "<redacted>",
                "nullable": "<redacted>"
            }))
        );
        assert_eq!(*sanitizer.paths.lock().unwrap(), ["/tokens", "/nullable"]);
    }

    #[test]
    fn structural_shape_mismatches_fail_the_complete_projection_closed() {
        let policy = PolicySanitizer::default();
        let object_expected = invocation_arguments(json!({
            "request": [{"id": 7, "children": []}]
        }));
        assert!(
            sanitize_serialized_arguments(&method(false), &object_expected, &policy).is_omitted()
        );

        let sequence_expected =
            MethodDescriptor::new(MethodId::new(0), "sequence", http_operation())
                .unwrap()
                .with_sensitivity(MethodSensitivity::new(
                    vec![SensitiveArgument::new("tokens", secret_sequence)],
                    None,
                ));
        let wrong_sequence = invocation_arguments(json!({"tokens": "not-an-array"}));
        assert!(
            sanitize_serialized_arguments(&sequence_expected, &wrong_sequence, &policy)
                .is_omitted()
        );
    }

    #[test]
    fn optional_dtos_and_nested_sequences_preserve_their_json_structure() {
        let descriptor = MethodDescriptor::new(MethodId::new(0), "optional", http_operation())
            .unwrap()
            .with_sensitivity(MethodSensitivity::new(
                vec![
                    SensitiveArgument::new("child", optional_child),
                    SensitiveArgument::new("children", optional_children),
                ],
                None,
            ));
        let policy = PolicySanitizer::default();
        let present = invocation_arguments(json!({
            "child": {"name": "one", "secret": "hidden"},
            "children": [{"name": "two", "secret": "hidden"}]
        }));
        assert_eq!(
            sanitize_serialized_arguments(&descriptor, &present, &policy).as_value(),
            Some(&json!({
                "child": {"name": "one", "secret": "<redacted>"},
                "children": [{"name": "two", "secret": "<redacted>"}]
            }))
        );

        let absent = invocation_arguments(json!({"child": null, "children": null}));
        assert!(sanitize_serialized_arguments(&descriptor, &absent, &policy).is_omitted());
    }

    #[test]
    fn fixed_sequences_require_the_declared_length_and_global_array_limit() {
        let descriptor = MethodDescriptor::new(MethodId::new(0), "fixed", http_operation())
            .unwrap()
            .with_sensitivity(MethodSensitivity::new(
                vec![SensitiveArgument::new("tokens", fixed_secrets)],
                None,
            ));
        let policy = PolicySanitizer::default();
        let valid = invocation_arguments(json!({"tokens": ["one", "two"]}));
        assert_eq!(
            sanitize_serialized_arguments(&descriptor, &valid, &policy).as_value(),
            Some(&json!({"tokens": "<redacted>"}))
        );
        let wrong_length = invocation_arguments(json!({"tokens": ["one"]}));
        assert!(sanitize_serialized_arguments(&descriptor, &wrong_length, &policy).is_omitted());

        let oversized =
            MethodDescriptor::new(MethodId::new(0), "oversized-fixed", http_operation())
                .unwrap()
                .with_sensitivity(MethodSensitivity::new(
                    vec![SensitiveArgument::new("tokens", oversized_fixed_secrets)],
                    None,
                ));
        let oversized_values = invocation_arguments(json!({"tokens": vec!["x"; 33]}));
        assert!(sanitize_serialized_arguments(&oversized, &oversized_values, &policy).is_omitted());
    }

    struct PanicSanitizer;

    impl Sanitizer for PanicSanitizer {
        fn sanitize(&self, _context: SanitizationContext<'_>) -> Sanitization {
            panic!("policy panic must not escape")
        }
    }

    struct PanicLimits;

    impl Sanitizer for PanicLimits {
        fn sanitize(&self, _context: SanitizationContext<'_>) -> Sanitization {
            Sanitization::Omit
        }

        fn limits(&self) -> ProjectionLimits {
            panic!("limits panic must not escape")
        }
    }

    #[test]
    fn policy_panics_and_missing_schemas_fail_closed() {
        let arguments = invocation_arguments(json!({"request": {"id": 7}}));
        assert!(
            sanitize_serialized_arguments(&method(false), &arguments, &PanicSanitizer).is_omitted()
        );

        let missing = MethodDescriptor::new(MethodId::new(0), "missing", http_operation()).unwrap();
        assert!(
            sanitize_serialized_arguments(&missing, &arguments, &PolicySanitizer::default())
                .is_omitted()
        );

        let panicking = MethodDescriptor::new(MethodId::new(0), "panicking", http_operation())
            .unwrap()
            .with_sensitivity(MethodSensitivity::new(
                vec![SensitiveArgument::new("request", panic_shape)],
                None,
            ));
        assert!(
            sanitize_serialized_arguments(&panicking, &arguments, &PolicySanitizer::default())
                .is_omitted()
        );
        assert!(
            sanitize_serialized_arguments(&method(false), &arguments, &PanicLimits).is_omitted()
        );
    }

    #[test]
    fn every_projection_limit_fails_the_complete_value_closed() {
        let test_cases = [
            ProjectionLimits::default().with_max_depth(1),
            ProjectionLimits::default().with_max_nodes(2),
            ProjectionLimits::default().with_max_array_items(1),
            ProjectionLimits::default().with_max_string_bytes(3),
            ProjectionLimits::default().with_max_output_bytes(4),
        ];
        let arguments = invocation_arguments(json!({
            "request": {
                "id": "long",
                "password": "secret",
                "children": [
                    {"name": "first", "secret": "one"},
                    {"name": "second", "secret": "two"}
                ]
            }
        }));

        for limits in test_cases {
            let policy = PolicySanitizer::default().with_limits(limits);
            assert!(
                sanitize_serialized_arguments(&method(false), &arguments, &policy).is_omitted()
            );
        }
    }

    #[test]
    fn response_input_limit_is_enforced_before_policy_dispatch() {
        let sanitizer = RecordingSanitizer {
            limits: Some(ProjectionLimits::default().with_max_input_bytes(16)),
            ..RecordingSanitizer::default()
        };
        let bytes = br#"                    {"name":"ok","secret":"x"}"#;

        assert!(
            sanitize_response(
                &method(true),
                bytes,
                Some(ProjectionDirection::Serialize),
                &sanitizer,
            )
            .is_omitted()
        );
        assert!(sanitizer.paths.lock().unwrap().is_empty());
    }

    #[test]
    fn response_requires_declared_origin_valid_json_and_response_schema() {
        let policy = PolicySanitizer::default();
        let descriptor = method(true);
        let bytes = br#"{"name":"visible","secret":"hidden","unknown":1}"#;

        let projected = sanitize_response(
            &descriptor,
            bytes,
            Some(ProjectionDirection::Serialize),
            &policy,
        );
        assert_eq!(
            projected.as_value(),
            Some(&json!({"name": "visible", "secret": "<redacted>"}))
        );
        assert!(sanitize_response(&descriptor, bytes, None, &policy).is_omitted());
        assert!(
            sanitize_response(
                &descriptor,
                b"not-json",
                Some(ProjectionDirection::Serialize),
                &policy,
            )
            .is_omitted()
        );
        assert!(
            sanitize_response(
                &descriptor,
                br#""not-an-object""#,
                Some(ProjectionDirection::Serialize),
                &policy,
            )
            .is_omitted()
        );
        assert!(
            sanitize_response(
                &method(false),
                bytes,
                Some(ProjectionDirection::Serialize),
                &policy,
            )
            .is_omitted()
        );
    }

    #[test]
    fn replacement_values_are_bounded() {
        let replacement = PolicySanitizer::default().with_rule(
            SensitivityKind::SECRET,
            Sanitization::Replace(json!({"safe": true})),
        );
        let arguments = invocation_arguments(json!({
            "request": {"id": 1, "password": "x", "children": []}
        }));
        let projected = sanitize_serialized_arguments(&method(false), &arguments, &replacement);
        assert_eq!(
            projected.as_value(),
            Some(&json!({
                "request": {"id": 1, "password": {"safe": true}, "children": []}
            }))
        );

        let oversized = PolicySanitizer::default()
            .with_limits(ProjectionLimits::default().with_max_string_bytes(3))
            .with_rule(
                SensitivityKind::SECRET,
                Sanitization::Replace(json!({"s": "too-long"})),
            );
        assert!(sanitize_serialized_arguments(&method(false), &arguments, &oversized).is_omitted());
    }

    #[test]
    fn deeply_nested_classified_values_are_checked_before_policy_dispatch() {
        let descriptor = MethodDescriptor::new(MethodId::new(0), "classified", http_operation())
            .unwrap()
            .with_sensitivity(MethodSensitivity::new(
                vec![SensitiveArgument::new("secret", secret)],
                None,
            ));
        let arguments = invocation_arguments(json!({
            "secret": {"nested": {"value": "hidden"}}
        }));
        let policy =
            PolicySanitizer::default().with_limits(ProjectionLimits::default().with_max_depth(1));

        assert!(sanitize_serialized_arguments(&descriptor, &arguments, &policy).is_omitted());
    }
}
