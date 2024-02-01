#[macro_export]
macro_rules! krpc_server {
    (
    $package:expr,
    $name:ident,
    $version:expr,
    $(async fn $method:ident (&$self:ident $(,$req:ident : $reqType:ty)* ) -> Result<$resType:ty>  { $($code:tt)* })*) => {
        impl $name {
            $(async fn $method (&$self $(,$req : $reqType)* ) -> krpc_common::Response<$resType> { $($code)* })*

            async fn prv_invoke (&self, mut param : krpc_common::KrpcMsg) -> krpc_common::KrpcMsg {
                $(if &param.method_name[..] == stringify!($method) {
                    param.res = match serde_json::from_slice(param.req.as_bytes()){
                        Ok(req) => {
                            let req : Vec<String> = req;
                            let mut idx = 0;
                            $(
                                let $req : $reqType = serde_json::from_slice(req[idx].as_bytes()).unwrap();
                                idx += 1;
                            )*
                            let res = self.$method(
                                $(
                                    $req,
                                )*
                            ).await;
                            match res {
                                Ok(res) => {
                                    let res = serde_json::to_string(&res);
                                    match res {
                                        Ok(res) => Ok(res),
                                        Err(err) => Err(krpc_common::RpcError::Server(err.to_string()))
                                    }
                                },
                                Err(info) => Err(krpc_common::RpcError::Method(info))
                            }
                        },
                        Err(err) => Err(krpc_common::RpcError::Server(err.to_string()))
                   };
                })*
                return param;
            }
        }
        impl krpc_common::RpcServer for $name {
            fn invoke (&self, param : krpc_common::KrpcMsg) -> krpc_common::KrpcFuture<krpc_common::KrpcMsg> {
                let rpc = self.clone();
                Box::pin(async move {rpc.prv_invoke(param).await})
            }
            fn get_info(&self) -> (&str , &str , &str) {
               ($package ,stringify!($name) , $version )
            }
        }
    }
}

#[macro_export]
macro_rules! krpc_client {
    (
    $cli:ident,
    $package:expr,
    $name:ident,
    $version:expr,
    $(async fn $method:ident (&$self:ident $(,$req:ident : $reqType:ty)* ) -> Result<$resType:ty> )*) => {
        impl $name {
            $(async fn $method (&$self $(,$req : $reqType)*) -> Result<$resType,krpc_common::RpcError> {
                let mut req_vec : Vec<String> = vec![];
                $(
                    let mut res_str = serde_json::to_string(&$req);
                    if let Err(err) = res_str {
                        return Err(krpc_common::RpcError::Client(err.to_string()));
                    }
                    req_vec.push(res_str.unwrap());
                )*
                let req_str = serde_json::to_string(&req_vec).unwrap();
                let msg = krpc_common::KrpcMsg::new(
                    "unique_identifier".to_string(),
                    $version.to_string(),
                    $package.to_owned() + "." + stringify!($name),
                    stringify!($method).to_string(),
                    req_str,
                    Err(krpc_common::RpcError::Null)
                );
                let res : Result<$resType,krpc_common::RpcError> = $cli.invoke::<$resType>(msg).await;
                return res;
            })*
        }
    }
}
