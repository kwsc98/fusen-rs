use serde::{de::DeserializeSeed, Deserialize, Deserializer, Serialize};

pub struct KrpcClient {
    addr: String,
}

#[derive(Serialize, Deserialize)]
pub struct KrpcRequest<Req, Res> {
    #[serde(default)]
    pub req: Req,
    #[serde(default)]
    pub res: Option<Res>,
}

impl KrpcClient {
    pub fn build(addr: String) -> KrpcClient {
        return KrpcClient { addr };
    }

    pub async fn invoke<Req, Res>(&mut self, req: KrpcRequest<Req, Res>) -> Res
    where
        Req: Send + Sync + 'static + Serialize,
        Res: Send + Sync + 'static + Serialize + for<'a> Deserialize<'a> + Default,
    {
        let de = serde_json::to_string(&req).unwrap();
        println!("{}", de);
        return req.res.unwrap();
    }
}
