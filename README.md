
# `fusen-rust` 一个最像RPC框架的Rust-RPC框架
fusen-rust是一个高性能，轻量级的rpc框架，通过使用Rust宏来解决目前主流rpc框架使用复杂，性能低等问题，不需要通过脚本和脚手架生成rpc调用代码，通过宏来进行编译期"反射"来实现高性能的调用，来满足rpc调用的简易性，同时支持Dubbo3服务的注册发现和互相调用;


## 快速开始

### Common InterFace
```rust
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ReqDto {
    pub str: String,
}

//#[rpc_trait(package = "org.apache.dubbo.springboot.demo", version = "1.0.0")]
#[rpc_trait(package = "org.apache.dubbo.springboot.demo")]
pub trait DemoService {

    async fn sayHello(&self, name: String) -> String;

    async fn sayHelloV2(&self, name: ReqDto) -> ResDto;

}
```


### Server
```rust
#[derive(Clone)]
struct DemoServiceImpl {
    _db: String,
}

//#[fusen_server(package = "org.apache.dubbo.springboot.demo", version = "1.0.0")]
//设置包路径和版本
#[fusen_server(package = "org.apache.dubbo.springboot.demo")]
impl DemoService for DemoServiceImpl {
    async fn sayHello(&self, req: String) -> RpcResult<String> {
        info!("res : {:?}", req);
        return Ok("Hello ".to_owned() + &req);
    }
    async fn sayHelloV2(&self, req: ReqDto) -> RpcResult<ResDto> {
        info!("res : {:?}", req);
        return Ok(ResDto {
            str: "Hello ".to_owned() + &req.str + " V2",
        });
    }
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    fusen_common::init_log();
    let server = DemoServiceImpl {
        _db: "我是一个DB数据库".to_string(),
    };
    FusenServer::build(
        //设置注册中心配置
        RegisterBuilder::new(
            &format!("127.0.0.1:{}", "2181"),
            "default",
            RegisterType::ZooKeeper,
        ),
        "8081",
    )
    //注册rpc服务
    .add_fusen_server(Box::new(server))
    .run()
    .await;
}
```

### Client
```rust
//初始化RpcClient
lazy_static! {
    static ref CLI: FusenClient = FusenClient::build(
        //设置注册中心配置
        RegisterBuilder::new(
        &format!("127.0.0.1:{}", "2181"),
        "default",
        RegisterType::ZooKeeper,
    ));
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    fusen_common::init_log();
    let client = DemoServiceRpc::new(&CLI);
    let res = client.sayHello("world".to_string()).await;
    info!("{:?}", res);
    let res = client
        .sayHelloV2(ReqDto {
            str: "world".to_string(),
        })
        .await;
    info!("{:?}", res);
}

```

### Dubbo3
本项目同时兼容dubbo3协议，可以很方便的与Java版本的Dubbo3项目通过接口暴露的方式进行服务注册发现和互调。

Rust的Server和Client完全不用改造就如上示例即可。

Java版本的Dubbo3项目，代码层面不需要改造，只需要添加一些依赖和配置（因Dubbo3使用接口暴露的方式默认不支持json序列化协议，而是采用fastjson2的二进制序列化格式，所以这里我们需手动添加fastjson1的支持）

这里我们使用duboo3的官方示例dubbo-samples-spring-boot项目进行演示
https://github.com/apache/dubbo-samples

首先我们需要把Server和Client的服务的pom.xml都添加fastjson1的maven依赖
```java
<dependency>
    <groupId>org.apache.dubbo</groupId>
    <artifactId>dubbo-serialization-fastjson</artifactId>
    <version>2.7.23</version>
</dependency>
```


### Java-Server
```java
@DubboService
public class DemoServiceImpl implements DemoService {

    @Override
    public String sayHello(String name) {
        return "Hello " + name;
    }
}
```

### Server-application.yml
```java
dubbo:
  application:
    name: dubbo-springboot-demo-provider
  protocol:
    name: tri
    port: 50052
    //添加fastjson的支持
    prefer-serialization: fastjson
  registry:
    address: zookeeper://${zookeeper.address:127.0.0.1}:2181
```

### Java-Client
```java
@Component
public class Task implements CommandLineRunner {
    @DubboReference
    private DemoService demoService;

    @Override
    public void run(String... args) throws Exception {
        String result = demoService.sayHello("world");
        System.out.println("Receive result ======> " + result);

        new Thread(()-> {
            while (true) {
                try {
                    Thread.sleep(1000);
                    System.out.println(new Date() + " Receive result ======> " + demoService.sayHello("world"));
                } catch (InterruptedException e) {
                    e.printStackTrace();
                    Thread.currentThread().interrupt();
                }
            }
        }).start();
    }
}
```

### Client-application.yml
```java
dubbo:
  application:
    name: dubbo-springboot-demo-consumer
  registry:
    address: zookeeper://${zookeeper.address:127.0.0.1}:2181
```
