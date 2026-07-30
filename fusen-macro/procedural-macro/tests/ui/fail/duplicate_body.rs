use fusen_procedural_macro::RpcMessage;

#[derive(RpcMessage)]
struct DuplicateBody {
    #[rpc(body)]
    first: String,
    #[rpc(body)]
    second: String,
}

fn main() {}
