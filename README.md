
# `fusen-rs` 一个最像RPC框架的Rust-RPC框架
fusen-rust是一个高性能，轻量级的微服务框架，通过使用Rust宏来解决目前主流rpc框架使用复杂，性能低等问题，不需要通过脚本和脚手架生成RPC调用代码，通过宏来进行编译期"反射"来实现高性能的调用，满足RPC调用的简易性，同时支持Dubbo3,SpringCloud微服务生态可以与Java项目进行服务注册发现与互相调用.


## 快速开始

### Common InterFace
```rust
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ReqDto {
    pub str: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ResDto {
    pub str: String,
}

#[fusen_trait(package = "org.apache.dubbo.springboot.demo")]
#[asset(spring_cloud = "service-provider")]
pub trait DemoService {
    async fn sayHello(&self, name: String) -> String;
    
    #[asset(path="/sayHelloV2-http",method = POST)]
    async fn sayHelloV2(&self, name: ReqDto) -> ResDto;

    #[asset(path="/divide",method = GET)]
    async fn divideV2(&self, a: i32, b: i32) -> String;
}
```


### Server
```rust
#[derive(Clone, Debug)]
struct DemoServiceImpl {
    _db: String,
}

#[fusen_server(package = "org.apache.dubbo.springboot.demo")]
impl DemoService for DemoServiceImpl {

    async fn sayHello(&self, req: String) -> FusenResult<String> {
        info!("res : {:?}", req);
        return Ok("Hello ".to_owned() + &req);
    }
    #[asset(path="/sayHelloV2-http",method = POST)]
    async fn sayHelloV2(&self, req: ReqDto) -> FusenResult<ResDto> {
        info!("res : {:?}", req);
        return Ok(ResDto {
            str: "Hello ".to_owned() + &req.str + " V2",
        });
    }

    #[asset(path="/divide",method = GET)]
    async fn divideV2(&self, a: i32, b: i32) -> FusenResult<String> {
        info!("res : a={:?},b={:?}", a, b);
        Ok((a + b).to_string())
    }
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    fusen_common::logs::init_log();
    let server = DemoServiceImpl {
        _db: "我是一个DB数据库".to_string(),
    };
    //支持多协议，多注册中心的接口暴露
    FusenServer::build()
        //初始化Fusen注册中心,同时支持Dubbo3协议与Fusen协议
        .add_register_builder(
            NacosConfig::builder()
                .server_addr("127.0.0.1:8848".to_owned())
                .app_name(Some("fusen-service".to_owned()))
                .server_type(fusen_rs::register::Type::Fusen)
                .build()
                .boxed(),
        )
        //初始化SpringCloud注册中心
        .add_register_builder(
            NacosConfig::builder()
                .server_addr("127.0.0.1:8848".to_owned())
                .app_name(Some("service-provider".to_owned()))
                .server_type(fusen_rs::register::Type::SpringCloud)
                .build()
                .boxed(),
        )
        //同时兼容RPC协议与HTTP协议
        .add_protocol(Protocol::HTTP("8081".to_owned()))
        .add_protocol(Protocol::HTTP2("8082".to_owned()))
        .add_fusen_server(Box::new(server))
        .run()
        .await;
}
```

### Client
```rust
lazy_static! {
    static ref CLI_FUSEN: FusenClient = FusenClient::build(
        NacosConfig::builder()
            .server_addr("127.0.0.1:8848".to_owned())
            .app_name(Some("fusen-client".to_owned()))
            .server_type(fusen_rs::register::Type::Fusen)
            .build()
            .boxed()
    );
    static ref CLI_DUBBO: FusenClient = FusenClient::build(
        NacosConfig::builder()
            .server_addr("127.0.0.1:8848".to_owned())
            .app_name(Some("dubbo-client".to_owned()))
            .server_type(fusen_rs::register::Type::Dubbo)
            .build()
            .boxed()
    );
    static ref CLI_SPRINGCLOUD: FusenClient = FusenClient::build(
        NacosConfig::builder()
            .server_addr("127.0.0.1:8848".to_owned())
            .app_name(Some("springcloud-client".to_owned()))
            .server_type(fusen_rs::register::Type::SpringCloud)
            .build()
            .boxed()
    );
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    fusen_common::logs::init_log();
    //进行Fusen协议调用HTTP2 + JSON
    let client = DemoServiceClient::new(&CLI_FUSEN);
    let res = client
        .sayHelloV2(ReqDto {
            str: "world".to_string(),
        })
        .await;
    info!("rev fusen msg : {:?}", res);

    //进行Dubbo3协议调用HTTP2 + GRPC
    let client = DemoServiceClient::new(&CLI_DUBBO);
    let res = client.sayHello("world".to_string()).await;
    info!("rev dubbo3 msg : {:?}", res);

    //进行SpringCloud协议调用HTTP1 + JSON
    let client = DemoServiceClient::new(&CLI_SPRINGCLOUD);
    let res = client.divideV2(1, 2).await;
    info!("rev springcloud msg : {:?}", res);
}
```

## Dubbo3
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
https://github.com/alibaba/spring-cloud-alibaba

Rust的Server和Client端的代码无需改造就如上示例即可。
Java的Server和Client端的代码也无需改造。直接启动即可。

### Server
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
		}
		else {
			return String.valueOf(a / b);
		}
	}
...
}
```

### Client
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
http://127.0.0.1:18083/divide-feign?a=1&b=2
```rust
2024-04-10T06:52:32.737307Z  INFO ThreadId(07) server: 33: res : a=1,b=2
```

测试curl ( curl => fusen-rust )

http://127.0.0.1:8081/divide?a=2&b=3

```rust
2024-04-10T06:54:26.436416Z  INFO ThreadId(512) server: 33: res : a=2,b=3
```

测试curl ( curl => fusen-rust )

curl --location --request POST 'http://127.0.0.1:8081/sayHelloV2-http' \
--header 'Content-Type: application/json' \
--header 'Connection: keep-alive' \
--data-raw '{
    "str" : "World"
}'

```rust
2024-04-10T07:02:50.138057Z  INFO ThreadId(03) server: 26: res : ReqDto { str: "World" }
```