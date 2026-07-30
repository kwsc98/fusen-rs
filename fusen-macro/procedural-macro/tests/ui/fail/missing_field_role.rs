use fusen_procedural_macro::RpcMessage;

#[derive(RpcMessage)]
struct MissingRole {
    id: String,
}

fn main() {}
