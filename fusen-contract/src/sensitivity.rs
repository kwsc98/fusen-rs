use crate::ContractError;

const MAX_SENSITIVITY_KIND_BYTES: usize = 64;

/// A validated classification attached to a value that may enter diagnostics.
///
/// Classifications are process-local policy metadata. They are never encoded into invocation traffic or
/// registry records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SensitivityKind(&'static str);

impl SensitivityKind {
    /// A value that is safe to retain in diagnostics.
    pub const PUBLIC: Self = Self("public");
    /// A login credential or equivalent authentication secret.
    pub const CREDENTIAL: Self = Self("credential");
    /// An access, refresh, session, or API token.
    pub const TOKEN: Self = Self("token");
    /// A telephone number.
    pub const PHONE: Self = Self("phone");
    /// An email address.
    pub const EMAIL: Self = Self("email");
    /// A user, account, device, or other identifying value.
    pub const IDENTIFIER: Self = Self("identifier");
    /// A generic secret that does not fit a narrower classification.
    pub const SECRET: Self = Self("secret");

    /// Creates a validated application-defined classification.
    ///
    /// The label must contain 1-64 ASCII letters, digits, `.`, `_`, or `-`.
    pub fn new(label: &'static str) -> Result<Self, ContractError> {
        if is_valid_kind(label) {
            Ok(Self(label))
        } else {
            Err(ContractError::InvalidSensitivityKind(label))
        }
    }

    /// Returns the stable classification label.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for SensitivityKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Lazily resolves the sensitivity shape of a Rust type.
///
/// A function pointer permits recursive DTOs without recursively constructing static values.
pub type SensitiveShapeResolver = fn() -> SensitiveShape;

/// The sensitivity metadata associated with one JSON value.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum SensitiveShape {
    /// The value's structure is unavailable and must be handled conservatively.
    Opaque,
    /// The complete value has one classification.
    Kind(SensitivityKind),
    /// An object whose named fields have independently resolved shapes in each Serde direction.
    Fields {
        /// Field names emitted when the value is serialized.
        serialize: &'static [SensitiveField],
        /// Field names accepted when the value is deserialized.
        deserialize: &'static [SensitiveField],
    },
    /// A nullable value that otherwise has the lazily resolved inner shape.
    Optional(SensitiveShapeResolver),
    /// A variable-length JSON array whose elements share one lazily resolved shape.
    Sequence(SensitiveShapeResolver),
    /// A fixed-length JSON array whose elements share one lazily resolved shape.
    FixedSequence {
        /// Lazily resolves the element shape.
        element: SensitiveShapeResolver,
        /// Required number of serialized elements.
        length: usize,
    },
}

/// Lazily resolved sensitivity metadata for one named DTO field.
#[derive(Clone, Copy)]
pub struct SensitiveField {
    name: &'static str,
    resolver: SensitiveShapeResolver,
}

impl SensitiveField {
    /// Creates metadata for one statically named field.
    ///
    /// Inside an inline static slice, wrap constructor calls in `const { ... }` so the compiler can
    /// promote the slice, for example `&[const { SensitiveField::new("id", resolver) }]`.
    pub const fn new(name: &'static str, resolver: SensitiveShapeResolver) -> Self {
        Self { name, resolver }
    }

    /// Returns this field's name in the associated JSON representation direction.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Resolves the field's sensitivity shape.
    pub fn shape(&self) -> SensitiveShape {
        (self.resolver)()
    }
}

impl std::fmt::Debug for SensitiveField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SensitiveField")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Supplies process-local sensitivity metadata for a Rust type's JSON representations.
///
/// Implementations should return [`SensitiveShape::Opaque`] when they cannot describe a value
/// safely. Container implementations preserve nullable and sequence structure while lazily
/// delegating to their element type.
pub trait SensitiveFields {
    /// Returns the sensitivity shape for this type.
    fn sensitive_shape() -> SensitiveShape;
}

/// Lazily resolved sensitivity metadata for one named invocation argument.
#[derive(Clone, Copy)]
pub struct SensitiveArgument {
    name: &'static str,
    resolver: SensitiveShapeResolver,
}

impl SensitiveArgument {
    /// Creates metadata for one statically named invocation argument.
    pub const fn new(name: &'static str, resolver: SensitiveShapeResolver) -> Self {
        Self { name, resolver }
    }

    /// Returns the argument's stable wire name.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Resolves the argument's sensitivity shape.
    pub fn shape(&self) -> SensitiveShape {
        (self.resolver)()
    }
}

impl std::fmt::Debug for SensitiveArgument {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SensitiveArgument")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Process-local sensitivity metadata for one service method.
///
/// This value does not participate in method identity, binding capabilities, discovery, or
/// registration.
#[derive(Clone)]
pub struct MethodSensitivity {
    arguments: Vec<SensitiveArgument>,
    response: Option<SensitiveShapeResolver>,
}

impl MethodSensitivity {
    /// Creates method metadata from ordered arguments and an optional response shape.
    pub fn new(
        arguments: Vec<SensitiveArgument>,
        response: Option<SensitiveShapeResolver>,
    ) -> Self {
        Self {
            arguments,
            response,
        }
    }

    /// Returns argument metadata in generated declaration order.
    pub fn arguments(&self) -> &[SensitiveArgument] {
        &self.arguments
    }

    /// Resolves the optional successful-response shape.
    pub fn response_shape(&self) -> Option<SensitiveShape> {
        self.response.map(|resolver| resolver())
    }
}

impl std::fmt::Debug for MethodSensitivity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MethodSensitivity")
            .field("arguments", &self.arguments)
            .field("has_response", &self.response.is_some())
            .finish()
    }
}

macro_rules! opaque_scalar {
    ($($type:ty),+ $(,)?) => {
        $(
            impl SensitiveFields for $type {
                fn sensitive_shape() -> SensitiveShape {
                    SensitiveShape::Opaque
                }
            }
        )+
    };
}

opaque_scalar!(
    (),
    bool,
    char,
    str,
    String,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    f32,
    f64
);

impl<T: SensitiveFields> SensitiveFields for Option<T> {
    fn sensitive_shape() -> SensitiveShape {
        SensitiveShape::Optional(T::sensitive_shape)
    }
}

impl<T: SensitiveFields> SensitiveFields for Vec<T> {
    fn sensitive_shape() -> SensitiveShape {
        SensitiveShape::Sequence(T::sensitive_shape)
    }
}

impl<T: SensitiveFields> SensitiveFields for [T] {
    fn sensitive_shape() -> SensitiveShape {
        SensitiveShape::Sequence(T::sensitive_shape)
    }
}

impl<T: SensitiveFields, const LENGTH: usize> SensitiveFields for [T; LENGTH] {
    fn sensitive_shape() -> SensitiveShape {
        SensitiveShape::FixedSequence {
            element: T::sensitive_shape,
            length: LENGTH,
        }
    }
}

impl<T: SensitiveFields + ?Sized> SensitiveFields for Box<T> {
    fn sensitive_shape() -> SensitiveShape {
        T::sensitive_shape()
    }
}

impl<T: SensitiveFields + ?Sized> SensitiveFields for std::sync::Arc<T> {
    fn sensitive_shape() -> SensitiveShape {
        T::sensitive_shape()
    }
}

impl<T: SensitiveFields + ?Sized> SensitiveFields for &T {
    fn sensitive_shape() -> SensitiveShape {
        T::sensitive_shape()
    }
}

impl<T: SensitiveFields + ?Sized> SensitiveFields for &mut T {
    fn sensitive_shape() -> SensitiveShape {
        T::sensitive_shape()
    }
}

fn is_valid_kind(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SENSITIVITY_KIND_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RecursiveDto;

    #[cfg(feature = "derive")]
    #[derive(serde::Serialize, crate::SensitiveFields)]
    struct DerivedDto {
        #[sensitive(kind = "token")]
        token: String,
    }

    impl SensitiveFields for RecursiveDto {
        fn sensitive_shape() -> SensitiveShape {
            const FIELDS: &[SensitiveField] = &[
                const { SensitiveField::new("id", SensitivityKind::identifier_shape) },
                const {
                    SensitiveField::new(
                        "children",
                        <Vec<Box<RecursiveDto>> as SensitiveFields>::sensitive_shape,
                    )
                },
            ];
            SensitiveShape::Fields {
                serialize: FIELDS,
                deserialize: FIELDS,
            }
        }
    }

    impl SensitivityKind {
        fn identifier_shape() -> SensitiveShape {
            SensitiveShape::Kind(Self::IDENTIFIER)
        }
    }

    #[test]
    fn kinds_validate_custom_labels() {
        assert_eq!(SensitivityKind::TOKEN.as_str(), "token");
        assert_eq!(
            SensitivityKind::new("application.customer-id").unwrap(),
            SensitivityKind::new("application.customer-id").unwrap()
        );
        for invalid in ["", "has space", "has/slash"] {
            assert_eq!(
                SensitivityKind::new(invalid),
                Err(ContractError::InvalidSensitivityKind(invalid))
            );
        }
        assert!(
            SensitivityKind::new(
                "this-classification-label-is-deliberately-longer-than-sixty-four-bytes"
            )
            .is_err()
        );
    }

    #[test]
    fn container_shapes_delegate_without_eager_recursive_construction() {
        let SensitiveShape::Fields {
            serialize,
            deserialize,
        } = RecursiveDto::sensitive_shape()
        else {
            panic!("recursive DTO should expose named fields");
        };
        assert_eq!(
            serialize
                .iter()
                .map(SensitiveField::name)
                .collect::<Vec<_>>(),
            ["id", "children"]
        );
        assert_eq!(
            deserialize
                .iter()
                .map(SensitiveField::name)
                .collect::<Vec<_>>(),
            ["id", "children"]
        );
        assert!(matches!(
            serialize[0].shape(),
            SensitiveShape::Kind(SensitivityKind::IDENTIFIER)
        ));

        let SensitiveShape::Sequence(child) = serialize[1].shape() else {
            panic!("container resolver should lazily resolve its recursive element");
        };
        let SensitiveShape::Fields {
            serialize: children,
            ..
        } = child()
        else {
            panic!("recursive element should expose named fields");
        };
        assert_eq!(children[1].name(), "children");
    }

    #[test]
    fn container_shapes_preserve_nullable_variable_and_fixed_structure() {
        let SensitiveShape::Optional(optional) =
            <Option<String> as SensitiveFields>::sensitive_shape()
        else {
            panic!("Option should preserve nullable structure");
        };
        assert!(matches!(optional(), SensitiveShape::Opaque));

        let SensitiveShape::Sequence(sequence) =
            <Vec<String> as SensitiveFields>::sensitive_shape()
        else {
            panic!("Vec should preserve sequence structure");
        };
        assert!(matches!(sequence(), SensitiveShape::Opaque));

        let SensitiveShape::FixedSequence { element, length } =
            <[String; 2] as SensitiveFields>::sensitive_shape()
        else {
            panic!("arrays should preserve their fixed length");
        };
        assert_eq!(length, 2);
        assert!(matches!(element(), SensitiveShape::Opaque));
    }

    #[test]
    fn method_metadata_resolves_arguments_and_response() {
        let metadata = MethodSensitivity::new(
            vec![SensitiveArgument::new(
                "phone",
                <String as SensitiveFields>::sensitive_shape,
            )],
            Some(<Option<String> as SensitiveFields>::sensitive_shape),
        );
        assert_eq!(metadata.arguments()[0].name(), "phone");
        assert!(matches!(
            metadata.arguments()[0].shape(),
            SensitiveShape::Opaque
        ));
        let Some(SensitiveShape::Optional(response)) = metadata.response_shape() else {
            panic!("optional response should preserve its nullable shape");
        };
        assert!(matches!(response(), SensitiveShape::Opaque));
    }

    #[cfg(feature = "derive")]
    #[test]
    fn derive_feature_reexports_the_macro_alongside_the_trait() {
        let dto = DerivedDto {
            token: String::new(),
        };
        assert!(dto.token.is_empty());
        let SensitiveShape::Fields {
            serialize,
            deserialize,
        } = DerivedDto::sensitive_shape()
        else {
            panic!("derived DTO should expose named fields");
        };
        assert_eq!(serialize[0].name(), "token");
        assert_eq!(deserialize[0].name(), "token");
        assert!(matches!(
            serialize[0].shape(),
            SensitiveShape::Kind(SensitivityKind::TOKEN)
        ));
    }
}
