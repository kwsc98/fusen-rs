use std::process::Output;

use futures::Future;
use krpc_common::{KrpcFuture, KrpcMsg, RpcServer};
use krpc_core::server::KrpcServer;
use krpc_macro::krpc_server;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
#[derive(Clone)]
struct TestServer {
    str: String,
}
#[derive(Clone)]
struct TestServer1 {
    str: String,
}

krpc_server! {
   TestServer
   async fn do_run1(&self,de : i32) -> i32 {
       return de;
   }
   async fn do_run2(&self,de : i32) -> i32 {
    return de + 1;
   }
}

krpc_server! {
    TestServer1
    async fn do_run1(&self,de : i32) -> i32 {
        return de+2;
    }
    async fn do_run2(&self,de : i32) -> i32 {
     return de + 3;
    }
 }

// impl RpcServer for TestServer1 {
//     fn invoke(&mut self, msg: krpc_common::KrpcMsg) -> KrpcFuture<KrpcMsg> {
//         Box::pin(async move { self.invoke(msg).await })
//     }
// }

#[tokio::main(worker_threads = 200)]
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
        .add_rpc_server(Box::new(server) as Box<dyn RpcServer>)
        .add_rpc_server(Box::new(server2) as Box<dyn RpcServer>)
        .run()
        .await;
}
