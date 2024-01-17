
# `krpc-rust` 一个最像RPC框架的Rust-RPC框架
刚刚学习Rust语言或者没怎么了解Rust-RPC框架的同学，可能以为又是一个标题党了，但实际上了解过这部分的同学都知道，目前来说主流的Rust-RPC框架和实际定义的RPC框架还是有着很大的差别。我们先看一下隔壁Java是如何实现的，就拿本项目Java版本 [krpc-java](https://github.com/kwsc98/krpc) 举例，有兴趣学习Java-RPC框架的同学别忘了点个Star~

# krpc

实现一个基于netty单路复用网络模型的rpc框架，支持spring-boot启动，支持zookeeper，nacos注册中心。<br/>

## 以SpringBoot的方式启动
### 接口示例
 ```java
public interface ExampeService {
    ResponseDTO doRun(RequestDTO requestDTO);
}
 ``` 
### Customer
配置application.properties <br/>
krpc.registeredPath = nacos://114.116.3.221:8848
 ```java
@KrpcResource(version = "1.0.1",timeout = 1000)
private ExampeService exampeService;
 ``` 

### Provider
配置application.properties <br/>
krpc.registeredPath = nacos://114.116.3.221:8848 <br/>
krpc.port = 8082
 ```java
@KrpcService(version = "1.0.1")
public class ExampeServiceImpl implements ExampeService {
    
    @Override
    public ResponseDTO doRun(RequestDTO requestDTO) {
        ResponseDTO responseDTO = new ResponseDTO();
        responseDTO.setDate(new Date(requestDTO.getDate().getTime() + (long) requestDTO.getNum() * 60 * 60 * 1000));
        return responseDTO;
    }
}
 ``` 

我们看到只需要定义一个接口，然后Server端来实现这个接口，Client端给接口加一个注解，就可以进行RPC的调用了，这是因为Java拥有一个大杀器就是运行时反射，可以很轻松的在运行时对类进行增强，但是同样这也是Java的一大缺点就是因为运行时存在导致程序执行降低，那么以高性能著称Rust当然不存在运行时，但因此也缺少了运行时反射这一功能，那么目前主流的Rust-RPC框架是怎么解决这个问题的？目前市面上有两大产品分别是阿里的Dubbo和字节的Volo，首先我们看 [Dubbo](https://cn.dubbo.apache.org/zh-cn/overview/quickstart/rust/) 怎么做的吧。


Dubbo 快速入门章节中介绍了Dubbo Rust的使用方法，其实主要是分为三个部分

第一部分定义接口，dubbo目前支持很多协议，其中还支持gRPC协议，其中Rust版本就是通过ProtoBuf协议来实现接口


![avatar](http://s7eyya91n.hb-bkt.clouddn.com/WechatIMG176.jpg)


第二部则是通过定义文件实现相关的Rust代码，因为Rust没有运行时，所以Client调用时没有办法通过动态代理的方式生成client类，而dubbo的解决方法就是通过定义接口内容生成相关的Client调用代码，来"降低"使用者的使用成本。


![avatar](http://s7eyya91n.hb-bkt.clouddn.com/WechatIMG177.jpg)


第三部分则是编写相关的Server端代码逻辑，然后通过生成的Client代码进行RPC调用

![avatar](https://raw.githubusercontent.com/kwsc98/krpc-rust/main/readme_image/WechatIMG178.jpg?token=GHSAT0AAAAAACMIYVHENVDWPQHYDS5TY44GZNH43BQ)
![avatar](https://raw.githubusercontent.com/kwsc98/krpc-rust/main/readme_image/WechatIMG179.jpg?token=GHSAT0AAAAAACMIYVHEYYNOXBB7IFGAMT4AZNH43NA)

字节的Volo其实大体也是这个思路，通过IDL定义接口，然后通过脚手架脚本生成调用相关代码。有兴趣的同学可以看一下 [Volo-grpc](https://www.cloudwego.io/zh/docs/volo/volo-grpc/getting-started/) 的快速开始


![avatar](https://raw.githubusercontent.com/kwsc98/krpc-rust/main/readme_image/WechatIMG180.jpg?token=GHSAT0AAAAAACMIYVHFXD4QZVFH7DVVIAJ2ZNH43XQ)


总结下来就是通过接口定义使用脚本生成调用代码和服务接口，然后进行Server端业务实现和Client调用。

这样看下来Dubbo和Volo的实现，尤其对比Java版本的实现来说，是不是离真正的RPC框架还有很大的差距，包括还存在很多的问题，比如

1.  必须使用RPC接口的一些规范，比如响应什么错误码。
```rust
// #[async_trait]
#[async_trait]
impl Greeter for GreeterServerImpl {
    async fn greet(
        &self,
        request: Request<GreeterRequest>,
    ) -> Result<Response<GreeterReply>, dubbo::status::Status> {
        println!("GreeterServer::greet {:?}", request.metadata);

        Ok(Response::new(GreeterReply {
            message: "hello, dubbo-rust".to_string(),
        }))
    }
}
```

2.  Client的调用时请求体必须包装成代码生成的样式。
```rust
    let req = volo_gen::proto_gen::hello::HelloRequest {
        name: FastStr::from_static_str("Volo"),
    };
```

3.   关键是如果想修改请求响应字段或者新增接口时，那我们必现通过脚本来重新生成所有代码，包括Client端和Server端，我们都知道对于软件质量有要求的公司，在改动代码时都必需评估影响范围然后交由测试，那么是不是意味则我们有一些小的调整的话就得让测试进行全部的测试？

<br/>
那么Rust没有运行时反射是不是也是个"缺点"？目前来看确实是这样的，两大厂都只能交出这么一个不令我们满意的答案，Java有反射这个大杀器才在微服务领域独领风骚，那Rust有什么办法可以在微服务领域也挑战Java呢？那就不得不提Rust宏这个核弹级武器了。


## Rust 宏

Rust宏大家都戏称可以通过宏来实现另一种编程语言，可见宏的强大之处，我们都知道宏是作用于编译期，那么我们就拿宏来实现一个编译期的反射不就行了吗？事实也的确可以，说了上面那么大一段废话，下面接入正题，看一看 `krpc-rust` 是如果进行RPC调用的
## Server

 ```rust
use krpc_core::server::KrpcServer;
use krpc_macro::krpc_server;
use serde::{Deserialize, Serialize};

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

krpc_server! {
   TestServer,
   "1.0.0",
   async fn do_run1(&self,res : ReqDto) -> ResDto {
    println!("{:?}" ,res);
    return ResDto { str : "TestServer say hello 1".to_string()};
   }
   async fn do_run2(&self,res : ReqDto) -> ResDto {
    println!("{:?}" ,res);
    return ResDto { str : "TestServer say hello 2".to_string()};
   }
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    let server: TestServer = TestServer {
        _db: "我是一个DB数据库".to_string(),
    };
    KrpcServer::build()
        .set_port("8081")
        .add_rpc_server(Box::new(server))
        .run()
        .await;
} 

```
## Client
```rust
use krpc_core::client::KrpcClient;
use krpc_macro::krpc_client;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

lazy_static! {
    static ref CLI: KrpcClient = KrpcClient::build("http://127.0.0.1:8081".to_string());
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

krpc_client! {
   CLI,
   TestServer,
   "1.0.0",
   async fn do_run1(&self,res : ReqDto) -> ResDto
   async fn do_run2(&self,res : ReqDto) -> ResDto 
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    let client = TestServer;
    let res = client.do_run1(ReqDto{str : "client say hello 1".to_string()}).await;
    println!("{:?}",res);
    let res = client.do_run2(ReqDto{str : "client say hello 2".to_string()}).await;
    println!("{:?}",res);
}
```
我们直接运行一下来看

![avatar](https://raw.githubusercontent.com/kwsc98/krpc-rust/main/readme_image/WechatIMG182.jpg?token=GHSAT0AAAAAACMIYVHFZSXQGGUBZM2PAUUOZNH55JQ)

这是不是才是RPC框架因有的样子？看到这里的同学是不是得本项目点个Star感谢支持,这个项目是一个很好的学习项目，同时也希望通过这个项目能让Rust在微服务领域同样有所发展。得益于Rust零抽象成本的概念，本项目当然也以高性能为目标，那我们就简单做个压力测试呗，因为Dubbo目前开源的版本示例我弄了一会儿没跑起来...那么我们就和Volo比一下。

![keyword](https://img.shields.io/github/stars/kwsc98/krpc-rust.svg?style=social&label=Star&maxAge=2592000) 

本次压测机器是MacBook Pro M1 16 + 512
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

如果感兴趣的同学可以一起讨论学习，目前本项目还有需要工作需要做，比如说异常的处理，多组件的支持，服务的注册和发现，但是框架骨架已搭建并验证完毕，未来可期~