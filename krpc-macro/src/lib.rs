#[macro_export]
macro_rules! krpc_server {
    ($name:ident
    $(async fn $method:ident (&$self:ident, $req:ident : $reqType:ty ) ->  $resType:ty  { $($code:tt)* })*) => {
        impl $name {
            $(async fn $method (&$self, $req : $reqType) -> $resType { $($code)* })*

            async fn prv_invoke (&self, mut param : krpc_common::KrpcMsg) -> krpc_common::KrpcMsg {
                $(if &param.method_name[..] == stringify!($method) {
                   let res = self.$method (serde_json::from_slice(param.data.as_bytes()).unwrap()).await;
                   param.data = serde_json::to_string(&res).unwrap();
                })*
                return param;
            }
        }
        impl krpc_common::RpcServer for $name {
            fn invoke (&mut self, param : krpc_common::KrpcMsg) -> krpc_common::KrpcFuture<krpc_common::KrpcMsg> {
                let s = self.clone();
                Box::pin(async move { s.prv_invoke(param).await })
            }

            fn get_info(&self) -> (String) {
               return (stringify!($name).to_string())
            }
        }
    }
}

#[macro_export]
macro_rules! krpc_client {
    ($name:ident
    $(async fn $method:ident (&$self:ident, $req:ident : $reqType:ty ) ->  $resType:ty  { $($code:tt)* })*) => {
        impl $name {
            $(async fn $method (&$self, $req : $reqType) -> $resType { $($code)* })*

            async fn prv_invoke (&self, mut param : krpc_common::KrpcMsg) -> krpc_common::KrpcMsg {
                $(if &param.method_name[..] == stringify!($method) {
                   let res = self.$method (serde_json::from_slice(param.data.as_bytes()).unwrap()).await;
                   param.data = serde_json::to_string(&res).unwrap();
                })*
                return param;
            }
        }
        impl krpc_common::RpcServer for $name {
            async fn invoke (&self, mut param : krpc_common::KrpcMsg) -> krpc_common::KrpcMsg {
                param = self.prv_invoke(param).await;
                return param;
            }
        }
    }
}
