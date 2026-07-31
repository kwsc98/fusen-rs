use fusen_procedural_macro::method;

#[method(method = "GET", path = "/standalone")]
fn standalone() {}

fn main() {}
