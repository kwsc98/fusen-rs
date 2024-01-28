
# `krpc-rust` 一个最像RPC框架的Rust-RPC框架
krpc-rust是一个高性能，轻量级的rpc框架，通过使用Rust宏来解决目前主流rpc框架使用复杂，性能低等问题，不需要通过脚本和脚手架生成rpc调用代码，通过宏来进行编译期"反射"来实现高性能的调用，来满足rpc调用的简易性，同时支持服务的注册发现和断线重连等。


## 快速开始


### Server
```rust
#[derive(Serialize, Deserialize, Default, Debug)]
struct ReqDto {
    str: String,
}
#[derive(Serialize, Deserialize, Default)]
struct ResDto {
    str: String,
}
#[derive(Clone)]
struct TestServer {
    _db: String,
}
//通过宏声明Server
krpc_server! {
   TestServer,
   //定义版本号
   "1.0.0",
   //实现rpc接口（错误响应）
   async fn do_run1(&self,res : ReqDto) -> Result<ResDto> {
      println!("{:?}" ,res);
      return Err("错误".to_string());
   }
   //实现rpc接口（正常响应）
   async fn do_run2(&self,res : ReqDto) -> Result<ResDto> {
     println!("{:?}" ,res);
     return Ok(ResDto { str : "TestServer say hello 1".to_string()});
    }
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    //实例化Server
    let server: TestServer = TestServer {
        _db: "我是一个DB数据库".to_string(),
    };
    //启动rpc服务
    KrpcServer::build(
        //设置注册中心配置（地址，工作空间，注册中心类型）
        RegisterBuilder::new(
            &format!("127.0.0.1:{}", "2181"),
            "default",
            RegisterType::ZooKeeper,
        ),
        //设置服务端口
        "8081",
    )
    //注册服务
    .add_rpc_server(Box::new(server))
    .run()
    .await;
}

```

### Client
```rust
//初始化RPC-Client
lazy_static! {
    static ref CLI: KrpcClient = KrpcClient::build(
        //设置注册中心配置（地址，工作空间，注册中心类型）
        RegisterBuilder::new(
            &format!("127.0.0.1:{}", "2181"),
            "default",
            RegisterType::ZooKeeper,
        )
    );
}
#[derive(Serialize, Deserialize, Default, Debug)]
struct ReqDto {
    str: String,
}
#[derive(Serialize, Deserialize, Default,Debug)]
struct ResDto {
    str: String,
}
struct TestServer;

//通过宏声明Client
krpc_client! {
   CLI,
   TestServer,
   "1.0.0",
   async fn do_run1(&self,res : ReqDto) -> Result<ResDto>
   async fn do_run2(&self,res : ReqDto) -> Result<ResDto> 
} 

#[tokio::main(worker_threads = 512)]
async fn main() {
    //实例化rpc接口
    let client = TestServer;
    //直接进行调用
    let res = client.do_run1(ReqDto{str : "client say hello 1".to_string()}).await;
    println!("{:?}",res);
    let res = client.do_run2(ReqDto{str : "client say hello 2".to_string()}).await;
    println!("{:?}",res);
}
```


这是不是才是RPC框架因有的样子？看到这里的同学是不是得本项目点个Star感谢支持,这个项目是一个很好的学习项目，同时也希望通过这个项目能让Rust在微服务领域同样有所发展。得益于Rust零抽象成本的概念，本项目当然也以高性能为目标，那我们就简单做个压力测试呗，因为Dubbo目前开源的版本示例我弄了一会儿没跑起来...那么我们就和Volo比一下。


本次压测机器是MacBook Pro M2 16 + 512
压测内容是四百万请求，异步线程数client端和server端各512，因为RPC调用时IO密集型所有多开一些线程。下面是测试脚本

![avatar](https://raw.githubusercontent.com/kwsc98/krpc-rust/main/readme_image/WechatIMG187.jpg?token=GHSAT0AAAAAACMIYVHFV62AJFM4RYGYFIKEZNH6PGA)
<br/><br/>
`krpc-rust` 测试结果 四百万请求，平均47秒跑完，每秒8.5w+QTS！！！而且内存占用也比较稳定
<br/>
![avatar](https://raw.githubusercontent.com/kwsc98/krpc-rust/main/readme_image/WechatIMG186.jpg?token=GHSAT0AAAAAACMIYVHFVBUVZGGIG6R2YN34ZNH6QCA)

<br/>
只能说不愧是Rust,Java表示实名制羡慕...
<br/>
接着我们看Volo的表现。
<br/>
额。。。出现点状况，测试100并发的时候还挺好好使，但是压测时内存和耗时异常高，因为为了压测关掉了日志打印，那么打开日志再看一下，结果

![avatar](https://raw.githubusercontent.com/kwsc98/krpc-rust/main/readme_image/WechatIMG28.jpg?token=GHSAT0AAAAAACMIYVHFRC2VPY7XDYPX3KJMZNH6XIQ)

设置到500并发时，socket连接就出现了错误，500个请求只成功了139个，可能目前Volo还存在一些问题，不过影响不大，我们已经证明了Rust在微服务领域其实是有机会干掉Java的。

