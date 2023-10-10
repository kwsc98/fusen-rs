use std::task::{Poll, Context};

use hyper::{service::Service, Request, Body, Response};

use crate::protocol::server::TcpServer;

struct KrpcServer {
    tcp_server: TcpServer,
}

#[pin_project]
pub struct RoutesFuture(#[pin] axum::routing::future::RouteFuture<Body, std::convert::Infallible>);

struct KrpcHandler {

}

impl Service<Request<Body>> for KrpcHandler {

    type Response = Response<crate::BoxBody>;

    type Error = crate::Error;

    type Future = RoutesFuture;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
       
    }
}
