use krpc_core::{
    krpc_server, register::{RegisterBuilder, RegisterType}, server::KrpcServer
};
use serde::{Deserialize, Serialize};
use tracing::info;
use examples::TestServer;
use krpc_macro::{rpc_resources, rpc_server};

#[derive(Serialize, Deserialize, Default, Debug)]
struct ReqDto {
    str: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct ResDto {
    str: String,
}

#[derive(Clone)]
struct TestServerImpl {
    _db: String,
}

#[rpc_server(package = "com.krpc", version = "1.0.0")]
impl TestServer for TestServerImpl {
    async fn do_run1(&self, res1: examples::ReqDto, res2: examples::ResDto) -> examples::ResDto {
        todo!()
    }

    async fn do_run2(&self, res: examples::ReqDto) -> examples::ResDto {
        todo!()
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
