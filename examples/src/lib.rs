struct TestServer { client : & 'static krpc_core :: client :: KrpcClient }
impl TestServer {     #[allow(non_snake_case)] async fn     do_run1(& self, res1 : ReqDto, res2 : ResDto) -> Result < ResDto, RpcError     > {}
    #[allow(non_snake_case)]
async fn do_run2(& self, res : ReqDto) ->     Result < ResDto, RpcError > {} }