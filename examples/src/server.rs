use std::time::Duration;

use examples::{ReqDto, ResDto, TestServer};
use fusen::{
    fusen_common::{self, logs::get_uuid, server::Protocol, FusenResult},
    fusen_macro::fusen_server,
    register::{Directory, Info, RegisterBuilder, RegisterType, Resource},
    server::FusenServer,
};
use tokio::time::sleep;
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
    // let server: TestServerImpl = TestServerImpl {
    //     _db: "我是一个DB数据库".to_string(),
    // };
    // FusenServer::build()
    //     .add_register_builder(RegisterBuilder::new(
    //         &format!("127.0.0.1:{}", "2181"),
    //         "default",
    //         RegisterType::ZooKeeper,
    //     ))
    //     .add_protocol(Protocol::HTTP("8082".to_owned()))
    //     .add_protocol(Protocol::HTTP2("8081".to_owned()))
    //     .add_fusen_server(Box::new(server))
    //     .run()
    //     .await;
    let mut de = Directory::new().await;
    let mut de_c = de.clone();

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let mut ds = vec![];
            let uuid = get_uuid();
            ds.push(Resource::Client(Info {
                server_name: uuid.clone(),
                version: None,
                methods: vec![],
                ip: uuid,
                port: None,
            }));
            let uuid = get_uuid();

            ds.push(Resource::Client(Info {
                server_name: uuid.clone(),
                version: None,
                methods: vec![],
                ip: uuid,
                port: None,
            }));
            let uuid = get_uuid();

            ds.push(Resource::Client(Info {
                server_name: uuid.clone(),
                version: None,
                methods: vec![],
                ip: uuid,
                port: None,
            }));

            println!("{:?}", de.change(ds).await);
        }
    });
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        println!("{:?}", de_c.get().await);
    }
}
