use examples::{DemoService, ReqDto, ResDto};
use krpc_common::RpcResult;
use krpc_core::{
    register::{RegisterBuilder, RegisterType},
    server::KrpcServer,
};
use krpc_macro::rpc_server;
use tracing::info;

#[derive(Clone)]
struct DemoServiceImpl {
    _db: String,
}

#[rpc_server(package = "org.apache.dubbo.springboot.demo")]
impl DemoService for DemoServiceImpl {
    async fn sayHello(&self, req: String) -> RpcResult<String> {
        info!("res : {:?}", req);
        return Ok("Hello ".to_owned() + &req);
    }
    async fn sayHelloV2(&self, req: ReqDto) -> RpcResult<ResDto> {
        info!("res : {:?}", req);
        return Ok(ResDto {
            str: "Hello ".to_owned() + &req.str + " V2",
        });
    }
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    krpc_common::init_log();
    let server = DemoServiceImpl {
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
