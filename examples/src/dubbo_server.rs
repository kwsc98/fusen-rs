use krpc_core::{
    register::{RegisterBuilder, RegisterType},
    server::KrpcServer,
};
use krpc_macro::krpc_server;


#[derive(Clone)]
struct DemoService {
    _db: String,
}

krpc_server! {
   "org.apache.dubbo.springboot.demo",
   DemoService,
   "1.0.0",
   async fn sayHello(&self,name : String) -> Result<String> {
      println!("res : {:?}" ,name);
      return Ok("Hello ".to_owned() + &name);
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
            RegisterType::DubboZooKeeper,
        ),
        "8081",
    )
    .add_rpc_server(Box::new(server))
    .run()
    .await;
}
