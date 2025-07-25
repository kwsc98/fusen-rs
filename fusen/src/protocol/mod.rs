pub mod codec;
pub mod fusen;
pub mod http;

#[derive(Default, Debug)]
pub enum Protocol {
    Dubbo,
    SpringCloud(String),
    #[default]
    Fusen,
    Host(String),
}
