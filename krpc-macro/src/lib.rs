
#[macro_export]
macro_rules! krpc_server {
    ($name:ident
    $(async fn $method:ident (&$self:ident, $req:ident : $reqType:ty ) ->  $resType:ty  { $($code:tt)* })*) => {
        impl $name {
            $(async fn $method (&$self, $req : $reqType) -> $resType { $($code)* })*
            async fn invoke (&self, mut param : KrpcMsg) -> KrpcMsg {
                $(if &param.method_name[..] == stringify!($method) {
                   let res = self.$method (serde_json::from_slice(param.data.as_bytes()).unwrap()).await;
                   param.data = serde_json::to_string(&res).unwrap();
                })*
                return param;
            }
        }
    }
}



