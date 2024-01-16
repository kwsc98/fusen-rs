use std::f64::consts;

use krpc_common::RpcServer;
use krpc_core::server::KrpcServer;
use krpc_macro::krpc_server;
use serde::{Deserialize, Serialize};
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Serialize, Deserialize, Default)]
struct ReqDto {
    str: String,
}
impl ReqDto {
    fn add_randStr(&mut self) {
        self.str = krpc_core::common::get_uuid();
    }
}
#[derive(Clone)]
struct TestServer {
    str: String,
}
}
#[derive(Clone)]
struct TestServer1 {
    str: String,
}

krpc_server! {
   TestServer,
   "1.0.0",
   async fn do_run1(&self,de : ReqDto) -> ReqDto {
    return de;
   }
   async fn do_run2(&self,de : ReqDto) -> ReqDto {
    return de;
   }
}

krpc_server! {
    TestServer1,
    "1.0.0",
    async fn do_run1(&self,de : ReqDto) -> ReqDto {
        return de;
    }
    async fn do_run2(&self,de : ReqDto) -> ReqDto {
     return de;
    }
 }

 impl TestServer {
    async fn do_run3(&self,de : ReqDto) -> ReqDto {
        let de1 = self as *const TestServer ;
        let de1 = unsafe { (&*de1).do_run2(de) }.await;
        return de1;
    }
 }

#[tokio::main(worker_threads = 500)]
async fn main() {
    let server: TestServer = TestServer {
        str: "de".to_string(),
    };
    let server2: TestServer1 = TestServer1 {
        str: "de".to_string(),
    };
    // tracing_subscriber::registry().with(fmt::layer()).init();
    KrpcServer::build()
        .set_port("8081")
        .add_rpc_server(Box::new(server))
        .run()
        .await;
}
