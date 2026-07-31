#[derive(Clone, Copy)]
pub struct SensitivityKind;

impl SensitivityKind {
    pub const PUBLIC: Self = Self;
    pub const CREDENTIAL: Self = Self;
    pub const TOKEN: Self = Self;
    pub const PHONE: Self = Self;
    pub const EMAIL: Self = Self;
    pub const IDENTIFIER: Self = Self;
    pub const SECRET: Self = Self;

    pub fn new(_value: &'static str) -> Result<Self, InvalidKind> {
        Ok(Self)
    }
}

#[derive(Debug)]
pub struct InvalidKind;

pub type SensitiveShapeResolver = fn() -> SensitiveShape;

pub struct SensitiveField;

impl SensitiveField {
    pub const fn new(_name: &'static str, _resolver: SensitiveShapeResolver) -> Self {
        Self
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
