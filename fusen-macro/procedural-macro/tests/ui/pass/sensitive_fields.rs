extern crate self as fusen_rs;

use fusen_procedural_macro::SensitiveFields;

pub mod contract {
    #[derive(Clone, Copy)]
    pub struct SensitivityKind(&'static str);

    impl SensitivityKind {
        pub const PUBLIC: Self = Self("public");
        pub const CREDENTIAL: Self = Self("credential");
        pub const TOKEN: Self = Self("token");
        pub const PHONE: Self = Self("phone");
        pub const EMAIL: Self = Self("email");
        pub const IDENTIFIER: Self = Self("identifier");
        pub const SECRET: Self = Self("secret");

        pub fn new(value: &'static str) -> Result<Self, InvalidKind> {
            Ok(Self(value))
        }
    }

    #[derive(Debug)]
    pub struct InvalidKind;

    pub type SensitiveShapeResolver = fn() -> SensitiveShape;

    pub struct SensitiveField {
        pub name: &'static str,
        pub resolver: SensitiveShapeResolver,
    }

    impl SensitiveField {
        pub const fn new(name: &'static str, resolver: SensitiveShapeResolver) -> Self {
            Self { name, resolver }
        }
    }

    pub enum SensitiveShape {
        Opaque,
        Kind(SensitivityKind),
        Fields(&'static [SensitiveField]),
    }

    pub trait SensitiveFields {
        fn sensitive_shape() -> SensitiveShape;
    }

    impl SensitiveFields for String {
        fn sensitive_shape() -> SensitiveShape {
            SensitiveShape::Opaque
        }
    }

    impl<T: SensitiveFields> SensitiveFields for Option<T> {
        fn sensitive_shape() -> SensitiveShape {
            T::sensitive_shape()
        }
    }

    impl<T: SensitiveFields> SensitiveFields for Box<T> {
        fn sensitive_shape() -> SensitiveShape {
            T::sensitive_shape()
        }
    }
}

mod external {
    #[derive(serde::Serialize)]
    pub struct SameNameHolder<T>(pub std::marker::PhantomData<T>);

    impl<T: super::contract::SensitiveFields> super::contract::SensitiveFields for SameNameHolder<T> {
        fn sensitive_shape() -> super::contract::SensitiveShape {
            T::sensitive_shape()
        }
    }
}

#[derive(serde::Serialize, SensitiveFields)]
struct Profile {
    #[sensitive(kind = "phone")]
    phone: String,
}

#[derive(serde::Serialize)]
struct ThirdParty;

fn serialize_external<S>(_: &ThirdParty, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_unit()
}

#[derive(serde::Serialize, SensitiveFields)]
#[serde(rename_all = "camelCase")]
struct Request<T> {
    #[sensitive(kind = "public")]
    user_name: String,
    #[serde(rename = "profile_data")]
    profile: Profile,
    nested: Option<T>,
    #[sensitive(opaque)]
    external: ThirdParty,
    #[serde(serialize_with = "serialize_external")]
    #[sensitive(kind = "vendor.secret")]
    custom: ThirdParty,
}

#[derive(serde::Serialize, SensitiveFields)]
#[serde(rename_all(serialize = "kebab-case", deserialize = "snake_case"))]
struct SerializeSpecificRename {
    #[serde(rename(serialize = "account-id", deserialize = "account_id"))]
    account_identifier: String,
}

#[derive(serde::Serialize, SensitiveFields)]
struct Recursive {
    next: Option<Box<Recursive>>,
}

#[derive(serde::Serialize, SensitiveFields)]
#[serde(transparent)]
struct Transparent<T>(T);

#[derive(serde::Serialize, SensitiveFields)]
#[sensitive(kind = "identifier")]
struct Identifier(String);

#[derive(serde::Serialize, SensitiveFields)]
#[sensitive(opaque)]
enum OpaqueChoice {
    One,
    Two(String),
}

#[derive(serde::Serialize, SensitiveFields)]
#[sensitive(opaque)]
struct OpaqueFlatten {
    #[serde(flatten)]
    values: std::collections::BTreeMap<String, String>,
}

#[derive(serde::Serialize)]
struct Wrapper<T> {
    #[serde(skip)]
    marker: std::marker::PhantomData<T>,
}

impl<T> contract::SensitiveFields for Wrapper<T> {
    fn sensitive_shape() -> contract::SensitiveShape {
        contract::SensitiveShape::Opaque
    }
}

struct NotSensitive;

#[derive(serde::Serialize, SensitiveFields)]
struct WrapperRequest<T> {
    wrapped: Wrapper<T>,
}

trait HasValue {
    type Value;
}

struct ValueProvider;

impl HasValue for ValueProvider {
    type Value = String;
}

#[derive(serde::Serialize, SensitiveFields)]
#[serde(bound(serialize = "T::Value: serde::Serialize"))]
struct AssociatedRequest<T: HasValue> {
    value: T::Value,
}

#[derive(serde::Serialize, SensitiveFields)]
struct SameNameHolder<T> {
    value: external::SameNameHolder<T>,
}

fn main() {
    use contract::SensitiveFields as _;

    let _ = Request::<String>::sensitive_shape();
    let _ = Transparent::<String>::sensitive_shape();
    let _ = Identifier::sensitive_shape();
    let _ = OpaqueChoice::sensitive_shape();
    let _ = SerializeSpecificRename::sensitive_shape();
    let _ = Recursive::sensitive_shape();
    let _ = OpaqueFlatten::sensitive_shape();
    let _ = WrapperRequest::<NotSensitive>::sensitive_shape();
    let _ = AssociatedRequest::<ValueProvider>::sensitive_shape();
    let _ = SameNameHolder::<String>::sensitive_shape();
}
