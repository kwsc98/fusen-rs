use krpc_core::{
    register::{RegisterBuilder, RegisterType},
    server::KrpcServer,
};
use krpc_macro::krpc_server;
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Serialize, Deserialize, Default, Debug)]
struct ReqDto {
    name: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct ResDto {
    res: String,
}

#[derive(Clone)]
struct DemoService {
    _db: String,
}

krpc_server! {
   "org.apache.dubbo.springboot.demo",
   DemoService,
   "1.0.0",
   async fn sayHello(&self,req : String) -> Result<String> {
      info!("res : {:?}" ,req);
      return Ok("Hello ".to_owned() + &req);
   }
   async fn sayHelloV2(&self,req : ReqDto) -> Result<ResDto> {
      info!("res : {:?}" ,req);
      return Ok(ResDto{res :  "Hello ".to_owned() + &req.name + " V2"});
   }
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    krpc_common::init_log();
    let server: DemoService = DemoService {
        _db: "我是一个DB数据库".to_string(),
    };
    KrpcServer::build(
        RegisterBuilder::new(
            &format!("127.0.0.1:{}", "2181"),
            "default",
            RegisterType::ZooKeeper,
        ),
        "8081",
    )
    .add_rpc_server(Box::new(server))
    .run()
    .await;
}
