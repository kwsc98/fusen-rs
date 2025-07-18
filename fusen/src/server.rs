#[derive(Debug, Clone, Default)]
pub enum ServerType {
    Dubbo,
    SpringCloud,
    #[default]
    Fusen,
    Host(String),
}
