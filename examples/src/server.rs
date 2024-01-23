use krpc_core::server::KrpcServer;
use krpc_macro::krpc_server;
use serde::{Deserialize, Serialize}; 

#[derive(Serialize, Deserialize, Default, Debug)]
struct ReqDto {
    str: String,
}

#[derive(Serialize, Deserialize, Default)]
struct ResDto {
    str: String,
}

#[derive(Clone)]
struct TestServer {
    _db: String,
}

krpc_server! {
   TestServer,
   "1.0.0",
   async fn do_run1(&self,res : ReqDto) -> Result<ResDto> {
    //   println!("{:?}" ,res);
      return Err("错误".to_string());
   }
   async fn do_run2(&self,res : ReqDto) -> Result<ResDto> {
     println!("{:?}" ,res);
     return Ok(ResDto { str : "TestServer say hello 1".to_string()});
    }
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    let server: TestServer = TestServer {
        _db: "我是一个DB数据库".to_string(),
    };
    KrpcServer::build()
        .set_port("8081")
        .add_rpc_server(Box::new(server))
        .run()
        .await;
}
