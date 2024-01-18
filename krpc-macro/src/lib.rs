#[macro_export]
macro_rules! krpc_server {
    ($name:ident,
    $version:expr,
    $(async fn $method:ident (&$self:ident, $req:ident : $reqType:ty ) -> Result<$resType:ty>  { $($code:tt)* })*) => {
        impl $name {
            $(async fn $method (&$self, $req : $reqType) -> krpc_common::Response<$resType> { $($code)* })*

            async fn prv_invoke (&self, mut param : krpc_common::KrpcMsg) -> krpc_common::KrpcMsg {
                $(if &param.method_name[..] == stringify!($method) {
                   let res = self.$method (serde_json::from_slice(param.req.as_bytes()).unwrap()).await;
                   param.res = match res {
                       Ok(res) => Ok (serde_json::to_string(&res).unwrap()),
                       Err(info) => Err(info)
                   }
                })*
                return param;
            }
        }
        impl krpc_common::RpcServer for $name {
            fn invoke (&self, param : krpc_common::KrpcMsg) -> krpc_common::KrpcFuture<krpc_common::KrpcMsg> {
                let rpc = self.clone();
                Box::pin(async move {rpc.prv_invoke(param).await})
            }
            fn get_info(&self) -> (&str , &str) {
               (stringify!($name) , $version)
            }
        }
    }
}

#[macro_export]
macro_rules! krpc_client {
    (
    $cli:ident,
    $name:ident,
    $version:expr,
    $(async fn $method:ident (&$self:ident, $req:ident : $reqType:ty ) -> Result<$resType:ty> )*) => {
        impl $name {
            $(async fn $method (&$self, $req : $reqType) -> krpc_common::Response<$resType> {
                let res_str = serde_json::to_string(&$req).unwrap();
                let msg = krpc_common::KrpcMsg::new(
                    "unique_identifier".to_string(),
                    $version.to_string(),
                    stringify!($name).to_string(),
                    stringify!($method).to_string(),
                    res_str,
                    Err("empty".to_string())
                );
                let res : krpc_common::Response<$resType> = $cli.invoke::<$reqType,$resType>(msg).await;
                return res;
            })*
        }
    }
}
