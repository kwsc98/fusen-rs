use examples::{DemoService, ReqDto, ResDto};
use fusen::{
    fusen_common::{self, FusenResult, RpcServer},
    fusen_macro::{self, resource},
    register::{RegisterBuilder, RegisterType},
    server::FusenServer,
};
use fusen_macro::fusen_server;
use tracing::info;

#[derive(Clone)]
struct DemoServiceImpl {
    _db: String,
}

#[fusen_server(package = "org.apache.dubbo.springboot.demo")]
#[resource(id = "DemoService333" ,method = GET)]
impl DemoService for DemoServiceImpl {
    #[resource(path="/sayHello2",method = POST)]
    async fn sayHello(&self, req: String) -> FusenResult<String> {
        info!("res : {:?}", req);
        return Ok("Hello ".to_owned() + &req);
    }
    #[resource(id="dasd")]
    async fn sayHelloV2(&self, req: ReqDto) -> FusenResult<ResDto> {
        info!("res : {:?}", req);
        return Ok(ResDto {
            str: "Hello ".to_owned() + &req.str + " V2",
        });
    }
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    fusen_common::init_log();
    let server = DemoServiceImpl {
        _db: "我是一个DB数据库".to_string(),
    };
    let ds = server.get_info();
    println!("{:?}", ds);
    FusenServer::build(
        RegisterBuilder::new(
            &format!("127.0.0.1:{}", "2181"),
            "default",
            RegisterType::ZooKeeper,
        ),
        "8081",
    )
    .add_fusen_server(Box::new(server))
    .run()
    .await;
}
