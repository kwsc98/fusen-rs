use krpc_core::{
    register::{RegisterBuilder, RegisterType},
    server::KrpcServer,
};
use krpc_macro::krpc_server;
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Serialize, Deserialize, Default, Debug)]
struct ReqDto {
    str: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct ResDto {
    str: String,
}

#[derive(Clone)]
struct TestServer {
    _db: String,
}

krpc_server! {
   "com.krpc",
   TestServer,
   Some("1.0.0"),
   async fn do_run1(&self,req1 : ReqDto,req2 : ResDto) -> Result<ResDto> {
      info!("req1 : {:?} , req2 : {:?}" ,req1, req2);
      return Err("错误".to_string());
   }
   async fn do_run2(&self,req : ReqDto) -> Result<ResDto> {
      info!("{:?}" ,req);
      return Ok(ResDto { str : "TestServer say hello 2".to_string()});
    }
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    krpc_common::init_log();
    let server: TestServer = TestServer {
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
