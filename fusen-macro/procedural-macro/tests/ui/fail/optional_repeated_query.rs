use fusen_procedural_macro::RpcMessage;

#[derive(RpcMessage)]
struct OptionalRepeatedQuery {
    #[rpc(query)]
    tags: Option<Vec<String>>,
}

fn main() {}
