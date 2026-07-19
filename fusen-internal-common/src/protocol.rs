#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
/// HTTP wire behavior independent from endpoint addressing.
pub enum WireProtocol {
    /// SpringCloud-compatible JSON over HTTP/1.1.
    SpringCloud,
    /// Fusen JSON over HTTP/2.
    #[default]
    Fusen,
}
