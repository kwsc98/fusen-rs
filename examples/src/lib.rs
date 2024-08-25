use fusen_rs::{
    fusen_common::{
        date_util::get_now_date_time_as_millis,
        logs::{get_trade_id, get_uuid},
        FusenContext,
    },
    fusen_procedural_macro::{asset, fusen_trait, handler, Data},
    handler::aspect::Aspect,
};
use serde::{Deserialize, Serialize};
use tracing::{
    debug_span, error_span, info, info_span, instrument, span, trace_span, warn_span, Instrument,
    Span,
};
use uuid::timestamp::context;

#[derive(Serialize, Deserialize, Default, Debug, Data)]
pub struct ReqDto {
    str: String,
}

#[derive(Serialize, Deserialize, Default, Debug, Data)]
pub struct ResDto {
    str: String,
}

#[fusen_trait(id = "org.apache.dubbo.springboot.demo.DemoService")]
pub trait DemoService {
    async fn sayHello(&self, name: String) -> String;

    #[asset(path = "/sayHelloV2-http", method = POST)]
    async fn sayHelloV2(&self, name: ReqDto) -> ResDto;

    #[asset(path = "/divide", method = GET)]
    async fn divideV2(&self, a: i32, b: i32) -> String;
}

#[derive(Default, Data)]
pub struct LogAspect {
    level: String,
}

impl LogAspect {
    pub fn get_span(&self, trade_id: &str, path: &str) -> Span {
        match self.level.as_str() {
            "info" => info_span!("trade_span", trade_id = trade_id, path = path),
            "debug" => debug_span!("trade_span", trade_id = trade_id, path = path),
            "error" => error_span!("trade_span", trade_id = trade_id, path = path),
            "warn" => warn_span!("trade_span", trade_id = trade_id, path = path),
            _ => trace_span!("trade_span", trade_id = trade_id, path = path),
        }
    }
}

#[handler(id = "LogAspect")]
impl Aspect for LogAspect {
    async fn aroud(
        &self,
        filter: &'static dyn fusen_rs::filter::FusenFilter,
        mut context: FusenContext,
    ) -> Result<FusenContext, fusen_rs::Error> {
        let trade_id = context.get_meta_data().get_value("trade_id");
        let trade_id = match trade_id {
            Some(trade_id) => trade_id.to_owned(),
            None => {
                let trade_id = get_trade_id();
                context
                    .get_mut_request()
                    .get_mut_headers()
                    .insert("trade_id".to_owned(), trade_id.to_owned());
                trade_id
            }
        };
        let mut span = Span::current();
        println!("--{:?}---{:?}", span.is_disabled(), span.id());
        if span.is_disabled() {
            span = self.get_span(&trade_id, &context.get_context_info().get_path().get_key());
        }
        let result = tokio::spawn(
            async move {
                let start_time = get_now_date_time_as_millis();
                info!("start handler : {:?}", context);
                let context = filter.call(context).await;
                info!(
                    "end handler : {:?}",
                    get_now_date_time_as_millis() - start_time,
                );
                context
            }
            .instrument(span),
        )
        .await;
        result.unwrap()
    }
}
