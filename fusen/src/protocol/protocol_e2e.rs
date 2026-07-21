use crate::{
    client::{ClientOptions, FusenClientContextBuilder},
    error::FusenError,
    fusen_procedural_macro::{fusen_service, fusen_trait},
    server::FusenServerBuilder,
};
use std::{net::SocketAddr, time::Duration};
use tokio::{net::TcpStream, sync::oneshot};

#[fusen_trait(id = "protocol-e2e")]
#[crate::fusen_procedural_macro::asset(path = "/rpc", method = POST)]
trait ProtocolService {
    #[crate::fusen_procedural_macro::asset(path = "/items/{id}")]
    async fn echo(&self, id: String, values: Vec<i32>) -> Vec<i32>;
}

struct ProtocolServiceImpl;

#[fusen_service]
impl ProtocolService for ProtocolServiceImpl {
    async fn echo(&self, id: String, values: Vec<i32>) -> Result<Vec<i32>, FusenError> {
        assert_eq!(id, "a/b space");
        Ok(values)
    }
}

#[tokio::test]
async fn fusen_http2_round_trip_preserves_single_array_body() {
    let address = available_address();
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = FusenServerBuilder::new(address)
        .service((Box::new(ProtocolServiceImpl), None))
        .unwrap();
    let task = tokio::spawn(server.run_with_shutdown(async move {
        let _ = shutdown_receiver.await;
    }));
    wait_until_listening(address).await;

    let mut context = FusenClientContextBuilder::new().build().unwrap();
    let client = ProtocolServiceClient::init(
        &mut context,
        ClientOptions::direct(format!("http://{address}").parse().unwrap()),
    )
    .await
    .unwrap();
    let response = client
        .echo("a/b space".into(), vec![1, 2, 3])
        .await
        .unwrap();
    assert_eq!(response, vec![1, 2, 3]);
    client.close().await.unwrap();

    shutdown_sender.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("server shutdown exceeded its test deadline")
        .unwrap()
        .unwrap();
}

fn available_address() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

async fn wait_until_listening(address: SocketAddr) {
    for _ in 0..100 {
        if TcpStream::connect(address).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("server did not start listening");
}
