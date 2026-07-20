use fusen_procedural_macro::fusen_service;

trait Demo {}
struct Service;

#[fusen_service(id = "duplicate")]
impl Demo for Service {}

fn main() {}
