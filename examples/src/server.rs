use krpc_common::url_util::{decode_url, encode_url};
use krpc_core::{
    register::{RegisterBuilder, RegisterType},
    server::KrpcServer,
};
use krpc_macro::krpc_server;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Debug)]
struct ReqDto {
    str: String,
}

#[derive(Serialize, Deserialize, Default,Debug)]
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
   "1.0.0",
   async fn do_run1(&self,res1 : ReqDto,res2 : ResDto) -> Result<ResDto> {
      println!("res1 : {:?} , res2 : {:?}" ,res1, res2);
      return Err("错误".to_string());
   }
   async fn do_run2(&self,res : ReqDto) -> Result<ResDto> {
     println!("{:?}" ,res);
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
#[test]
fn test() {
    let str = "/dubbo/org.apache.dubbo.springboot.demo.DemoService/providers/tri%3A%2F%2F10.224.1.29%3A50052%2Forg.apache.dubbo.springboot.demo.DemoService%3Fapplication%3Ddubbo-springboot-demo-provider%26deprecated%3Dfalse%26dubbo%3D2.0.2%26dynamic%3Dtrue%26generic%3Dfalse%26interface%3Dorg.apache.dubbo.springboot.demo.DemoService%26methods%3DsayHello%26prefer.serialization%3Dfastjson2%2Chessian2%26release%3D3.3.0-beta.1%26service-name-mapping%3Dtrue%26side%3Dprovider%26timestamp%3D1706775452118";
    let str = decode_url(str).unwrap();
    println!("{:?}",str);
    println!("{:?}",encode_url(&str));
}