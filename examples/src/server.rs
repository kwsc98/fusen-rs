use examples::{ReqDto, ResDto, TestServer};
use fusen::fusen_common::url::UrlConfig;
use fusen::fusen_macro::asset;
use fusen::register::nacos::NacosConfig;
use fusen::{
    fusen_common::{self, server::Protocol, FusenResult},
    fusen_macro::fusen_server,
    server::FusenServer,
};
use tracing::info;

#[derive(Clone)]
struct TestServerImpl {
    _db: String,
}

#[fusen_server]
impl TestServer for TestServerImpl {
    async fn do_run1(&self, req1: ReqDto, req2: ReqDto) -> FusenResult<ResDto> {
        info!("req1 : {:?} , req1 : {:?}", req1, req2);
        return Ok(ResDto {
            str: "Hello ".to_owned() + &req1.str + " " + &req2.str + " V1",
        });
    }
    #[asset(path="/doRun2",method = POST)]
    async fn doRun2(&self, req: ReqDto) -> FusenResult<ResDto> {
        info!("res : {:?}", req);
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
        .add_register_builder(
            NacosConfig::builder()
                .server_addr("127.0.0.1:8848".to_owned())
                .app_name(Some("fusen-rust-server".to_owned()))
                .server_type(fusen::register::Type::SpringCloud)
                .build()
                .boxed(),
        )
        .add_protocol(Protocol::HTTP("8081".to_owned()))
        .add_protocol(Protocol::HTTP2("8082".to_owned()))
        .add_fusen_server(Box::new(server))
        .run()
        .await;
}
