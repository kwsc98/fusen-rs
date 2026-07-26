use fusen_procedural_macro::method;

#[method(idempotency = "safe")]
async fn detached() {}

fn main() {}
