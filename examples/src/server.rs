use examples::{ReqDto, ResDto, TestServer};
use fusen::fusen_common::url::UrlConfig;
use fusen::register::zookeeper::ZookeeperConfig;
use fusen::{
    fusen_common::{self, server::Protocol, FusenResult},
    fusen_macro::fusen_server,
    register::{RegisterBuilder, RegisterType},
    server::FusenServer,
};
use tracing::info;

#[derive(Clone)]
struct TestServerImpl {
    _db: String,
}

#[fusen_server(version = "1.0.0")]
impl TestServer for TestServerImpl {
    async fn do_run1(&self, req1: ReqDto, req2: ReqDto) -> FusenResult<ResDto> {
        info!("req1 : {:?} , req1 : {:?}", req1, req2);
        return Ok(ResDto {
            str: "Hello ".to_owned() + &req1.str + " " + &req2.str + " V1",
        });
    }
    async fn doRun2(&self, req: ReqDto) -> FusenResult<ResDto> {
        // info!("res : {:?}", req);
        return Ok(ResDto {
            str: "Hello ".to_owned() + &req.str + " V2",
        });
    }
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    fusen_common::logs::init_log();
    let server: TestServerImpl = TestServerImpl {
        _db: "我是一个DB数据库".to_string(),
    };
    FusenServer::build()
        .add_register_builder(RegisterBuilder::new(RegisterType::ZooKeeper(
            ZookeeperConfig::builder()
                .cluster("127.0.0.1:2181".to_owned())
                .build().boxed()
        )))
        .add_protocol(Protocol::HTTP("8082".to_owned()))
        .add_protocol(Protocol::HTTP2("8081".to_owned()))
        .add_fusen_server(Box::new(server))
        .run()
        .await;
}
