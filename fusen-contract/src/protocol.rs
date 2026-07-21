/// HTTP wire behavior independent from endpoint addressing.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WireProtocol {
    /// SpringCloud-compatible JSON over HTTP/1.1.
    SpringCloud,
    /// Fusen JSON over HTTP/2.
    #[default]
    Fusen,
}

impl WireProtocol {
    /// Returns the stable protocol identifier used in diagnostics and metadata.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SpringCloud => "spring-cloud",
            Self::Fusen => "fusen",
        }
    }
}

impl std::fmt::Display for WireProtocol {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}
