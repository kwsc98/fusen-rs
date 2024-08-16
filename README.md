
# `fusen-rs` 一个最像RPC框架的Rust-RPC框架

fusen-rust是一个高性能，轻量级的微服务框架，通过使用Rust宏来解决目前主流rpc框架使用复杂，性能低等问题，不需要通过脚本和脚手架生成RPC调用代码，通过宏来进行编译期"反射"来实现高性能的调用，满足RPC调用的简易性，同时支持Dubbo3,SpringCloud微服务生态可以与Java项目进行服务注册发现与互相调用,并且支持用户自定义组件等功能.

## 功能列表

- :white_check_mark: RPC调用抽象层(Rust宏)
- :white_check_mark: 多协议支持(HTTP1, HTTP2)
- :white_check_mark: 服务注册与发现(Nacos)
- :white_check_mark: 微服务生态兼容(Dubbo3, SpringCloud)
- :white_check_mark: 自定义组件(自定义负载均衡器,Aspect环绕通知组件)
- :white_check_mark: 配置中心(本地文件配置, Nacos)
- :white_check_mark: 优雅停机
- :construction: HTTP3协议支持

## 快速开始

### Common Interface

```rust
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ReqDto {
    pub str: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ResDto {
    pub str: String,
}

#[fusen_trait(id = "org.apache.dubbo.springboot.demo.DemoService")]
pub trait DemoService {
    async fn sayHello(&self, name: String) -> String;

    #[asset(path = "/sayHelloV2-http", method = POST)]
    async fn sayHelloV2(&self, name: ReqDto) -> ResDto;

    #[asset(path = "/divide", method = GET)]
    async fn divideV2(&self, a: i32, b: i32) -> String;
}
```

### Server

```rust
#[derive(Debug)]
struct DemoServiceImpl {
    _db: String,
}

#[fusen_server(id = "org.apache.dubbo.springboot.demo.DemoService")]
impl DemoService for DemoServiceImpl {
    async fn sayHello(&self, req: String) -> FusenResult<String> {
        info!("res : {:?}", req);
        Ok("Hello ".to_owned() + &req)
    }
    #[asset(path="/sayHelloV2-http",method = POST)]
    async fn sayHelloV2(&self, req: ReqDto) -> FusenResult<ResDto> {
        info!("res : {:?}", req);
        Ok(ResDto::default().str("Hello ".to_owned() + req.get_str() + " V2"))
    }
    #[asset(path="/divide",method = GET)]
    async fn divideV2(&self, a: i32, b: i32) -> FusenResult<String> {
        info!("res : a={:?},b={:?}", a, b);
        Ok((a + b).to_string())
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 16)]
async fn main() {
    fusen_common::logs::init_log();
    let server = DemoServiceImpl {
        _db: "我是一个DB数据库".to_string(),
    };
    FusenApplicationContext::builder()
        //使用配置文件进行初始化
        .init(get_config_by_file("examples/server-config.yaml").unwrap())
        .add_fusen_server(Box::new(server))
        .add_handler(ServerLogAspect.load())
        .build()
        .run()
        .await;
}
```

### Client

```rust
#[tokio::main(flavor = "multi_thread", worker_threads = 16)]
async fn main() {
    fusen_common::logs::init_log();
    let context = FusenApplicationContext::builder()
        //使用配置文件进行初始化
        .init(get_config_by_file("examples/client-config.yaml").unwrap())
        .add_handler(CustomLoadBalance.load())
        .add_handler(ClientLogAspect.load())
        .build();
    //直接当HttpClient调用HTTP1 + JSON
    let client = DemoServiceClient::new(Arc::new(
        context.client(Type::Host("127.0.0.1:8081".to_string())),
    ));
    let res = client
        .sayHelloV2(ReqDto::default().str("world".to_string()))
        .await;
    info!("rev host msg : {:?}", res);
    //通过Fusen进行服务注册与发现，并且进行HTTP2+JSON进行调用
    let client = DemoServiceClient::new(Arc::new(context.client(Type::Fusen)));
    let res = client
        .sayHelloV2(ReqDto::default().str("world".to_string()))
        .await;
    info!("rev fusen msg : {:?}", res);
    // //通过Dubbo进行服务注册与发现，并且进行HTTP2+Grpc进行调用
    let client = DemoServiceClient::new(Arc::new(context.client(Type::Dubbo)));
    let res = client
        .sayHelloV2(ReqDto::default().str("world".to_string()))
        .await;
    info!("rev dubbo msg : {:?}", res);
    //通过SpringCloud进行服务注册与发现，并且进行HTTP1+JSON进行调用
    let client = DemoServiceClient::new(Arc::new(context.client(Type::SpringCloud)));
    let res = client
        .sayHelloV2(ReqDto::default().str("world".to_string()))
        .await;
    info!("rev springcloud msg : {:?}", res);
}
```

## 自定义组件

微服务自定义组件包括, 负载均衡器, 服务熔断/限流组件, 前置后置请求处理器, 服务链路追踪等组件. 由于组件的定制化程度较高, 所以本项目参考AOP的概念提供了两种自定义组件,来提供灵活的请求处理。

### LoadBalance

负载均衡组件, LoadBalance提供一个select接口来实现用户自定义服务均衡配置。

```rust
#[handler(id = "CustomLoadBalance")]
impl LoadBalance for CustomLoadBalance {
    async fn select(
        &self,
        invokers: Arc<ResourceInfo>,
    ) -> Result<Arc<InvokerAssets>, fusen_rs::Error> {
        invokers
            .select()
            .ok_or("not find server : CustomLoadBalance".into())
    }
}
```

### Aspect

动态代理的概念相信大家都不陌生,这是Java对类进行增强的一种技术,而Spring框架利用此特性封装出了更高级的模型, 那就是AOP面先切面编程模型. 本组件就是参考了此模型,实现了环绕式通知模型, 用户可以基于此组件实现各种组件需求，比如说服务熔断/限流,请求的前置后置处理,链路追踪,请求响应时间监控等需求.

```rust
#[handler(id = "ClientLogAspect" )]
impl Aspect for ClientLogAspect {
    async fn aroud(
        &self,
        filter: &'static dyn fusen_rs::filter::FusenFilter,
        context: fusen_common::FusenContext,
    ) -> Result<fusen_common::FusenContext, fusen_rs::Error> {
        let start_time = get_now_date_time_as_millis();
        info!("client send request : {:?}", context);
        //执行RPC调用
        let context = filter.call(context).await;
        info!(
            "client receive response RT : {:?}ms : {:?}",
            get_now_date_time_as_millis() - start_time,
            context
        );
        context
    }
}
```

## Dubbo3

本项目同时兼容dubbo3协议，可以很方便的与Java版本的Dubbo3项目通过接口暴露的方式进行服务注册发现和互调。

Rust的Server和Client完全不用改造就如上示例即可。

Java版本的Dubbo3项目，代码层面不需要改造，只需要添加一些依赖和配置（因Dubbo3使用接口暴露的方式默认不支持json序列化协议，而是采用fastjson2的二进制序列化格式，所以这里我们需手动添加fastjson1的支持）

这里我们使用duboo3的官方示例dubbo-samples-spring-boot项目进行演示
<https://github.com/apache/dubbo-samples>

首先我们需要把Server和Client的服务的pom.xml都添加fastjson和nacos的maven依赖

```java
<dependency>
    <groupId>org.apache.dubbo</groupId>
    <artifactId>dubbo-serialization-fastjson</artifactId>
    <version>2.7.23</version>
</dependency>
<!-- registry dependency -->
<dependency>
    <groupId>com.alibaba.nacos</groupId>
    <artifactId>nacos-client</artifactId>
    <version>2.2.0</version>
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
    address: nacos://${nacos.address:127.0.0.1}:8848
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
    address: nacos://${nacos.address:127.0.0.1}:8848
```

## SpringCloud

同时本项目还拓展了HTTP接口可以当做一个WebServer框架，并且还支持了SpringCloud服务注册与发现，用户可以灵活的选择和切换需要暴露的协议，并且支持同时暴露。

这里我们使用spring-cloud-alibaba项目进行演示
<https://github.com/alibaba/spring-cloud-alibaba>

RustClient端调用SpringCloud需要将fusen_trait_id修改为目标服务id（application_name）

```rust
#[fusen_trait(id = "service-provider")]
```

Java的Server和Client端的代码也无需改造。直接启动即可。

### SpringCloud-Server

Provider启动类
package com.alibaba.cloud.examples.ProviderApplication

```java
//EchoController
@RestController
public class EchoController {
...
    @GetMapping("/divide")
    public String divide(@RequestParam Integer a, @RequestParam Integer b) {
        if (b == 0) {
          return String.valueOf(0);
        } else {
          return String.valueOf(a / b);
        }
    }
...
}
```

### SpringCloud-Client

Consumer启动类
package com.alibaba.cloud.examples.ConsumerApplication

```java
//TestController
@RestController
public class TestController {
...
    @GetMapping("/divide-feign")
    public String divide(@RequestParam Integer a, @RequestParam Integer b) {
        return echoClient.divide(a, b);
    }
...
}

```

测试curl ( curl => SpringCloud => fusen-rust )
<http://127.0.0.1:18083/divide-feign?a=1&b=2>

```rust
2024-04-10T06:52:32.737307Z  INFO ThreadId(07) server: 33: res : a=1,b=2
```

测试curl ( curl => fusen-rust )

<http://127.0.0.1:8081/divide?a=2&b=3>

```rust
2024-04-10T06:54:26.436416Z  INFO ThreadId(512) server: 33: res : a=2,b=3
```

测试curl ( curl => fusen-rust )

curl --location --request POST '<http://127.0.0.1:8081/sayHelloV2-http>' \
--header 'Content-Type: application/json' \
--header 'Connection: keep-alive' \
--data-raw '{
    "str" : "World"
}'

```rust
2024-04-10T07:02:50.138057Z  INFO ThreadId(03) server: 26: res : ReqDto { str: "World" }
```
