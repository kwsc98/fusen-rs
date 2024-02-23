use examples::{ReqDto, ResDto, TestServer};
use krpc_common::RpcResult;
use krpc_core::{
    register::{RegisterBuilder, RegisterType},
    server::KrpcServer,
};
use krpc_macro::rpc_server;
use tracing::info;

#[derive(Clone)]
struct TestServerImpl {
    _db: String,
}

#[rpc_server(package = "com.krpc", version = "1.0.0")]
impl TestServer for TestServerImpl {
    async fn do_run1(&self, req1: ReqDto, req2: ReqDto) -> RpcResult<ResDto> {
        info!("req1 : {:?} , req1 : {:?}", req1, req2);
        return Ok(ResDto {
            str: "Hello ".to_owned() + &req1.str + " " + &req2.str + " V1",
        });
    }

    async fn doRun2(&self, req: ReqDto) -> RpcResult<ResDto> {
        info!("res : {:?}", req);
        return Ok(ResDto {
            str: "Hello ".to_owned() + &req.str + " V2",
        });
    }
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    krpc_common::init_log();
    let server: TestServerImpl = TestServerImpl {
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
