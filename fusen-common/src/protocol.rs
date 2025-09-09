#[derive(Default, Debug, Clone)]
pub enum Protocol {
    Dubbo,
    SpringCloud(String),
    #[default]
    Fusen,
    Host(String),
}
