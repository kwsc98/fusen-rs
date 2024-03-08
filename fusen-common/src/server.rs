#[derive(Clone)]
pub enum Protocol {
    HTTP(String),
    HTTP2(String),
}
