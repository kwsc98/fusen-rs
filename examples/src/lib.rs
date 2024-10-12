use bytes::Bytes;
use fusen_rs::{
    filter::ProceedingJoinPoint,
    fusen_common::{self, date_util::get_now_date_time_as_millis, FusenContext, FusenRequest},
    fusen_procedural_macro::{asset, fusen_trait, handler, Data},
    handler::aspect::Aspect,
};
use opentelemetry::propagation::text_map_propagator::TextMapPropagator;
use opentelemetry::{trace::TraceContextExt, Context};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use serde::{Deserialize, Serialize};
use tracing::{debug_span, error, error_span, info, info_span, warn_span, Instrument, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

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

#[allow(dead_code)]
#[derive(Data)]
pub struct LogAspect {
    level: String,
    trace_context_propagator: TraceContextPropagator,
}

impl LogAspect {
    pub fn new(level: &str) -> Self {
        Self {
            level: level.to_owned(),
            trace_context_propagator: TraceContextPropagator::new(),
        }
    }
}

impl LogAspect {
    fn get_parent_span(&self, path: &str) -> Span {
        match self.get_level().as_str() {
            "info" => info_span!("begin_span", path = path),
            "debug" => debug_span!("begin_span", path = path),
            "warn" => warn_span!("begin_span", path = path),
            "error" => error_span!("begin_span", path = path),
            _ => tracing::trace_span!("begin_span", path = path),
        }
    }
    fn get_new_span(&self, context: Context, path: &str) -> Span {
        let span = match self.get_level().as_str() {
            "info" => info_span!(
                "trace_span",
                trace_id = context.span().span_context().trace_id().to_string(),
                path = path
            ),
            "debug" => debug_span!(
                "trace_span",
                trace_id = context.span().span_context().trace_id().to_string(),
                path = path
            ),
            "warn" => warn_span!(
                "trace_span",
                trace_id = context.span().span_context().trace_id().to_string(),
                path = path
            ),
            "error" => error_span!(
                "trace_span",
                trace_id = context.span().span_context().trace_id().to_string(),
                path = path
            ),
            _ => tracing::trace_span!(
                "trace_span",
                trace_id = context.span().span_context().trace_id().to_string(),
                path = path
            ),
        };
        span.set_parent(context);
        span
    }
}

#[handler(id = "LogAspect")]
impl Aspect for LogAspect {
    async fn aroud(
        &self,
        mut join_point: ProceedingJoinPoint,
    ) -> Result<fusen_common::FusenContext, fusen_rs::Error> {
        let mut span_context = self.get_trace_context_propagator().extract_with_context(
            &Span::current().context(),
            join_point.get_context().get_meta_data().get_inner(),
        );
        let mut first_span = None;
        if !span_context.has_active_span() {
            let span = self.get_parent_span(
                &join_point
                    .get_context()
                    .get_context_info()
                    .get_path()
                    .get_key(),
            );
            span_context = span.context();
            let _ = first_span.insert(span);
        }
        let span = self.get_new_span(
            span_context,
            &join_point
                .get_context()
                .get_context_info()
                .get_path()
                .get_key(),
        );
        let trace_id = span.context().span().span_context().trace_id().to_string();
        span.set_attribute("trace_id", trace_id.to_owned());
        if join_point
            .get_context()
            .get_meta_data()
            .get_value("traceparent")
            .is_none()
        {
            self.get_trace_context_propagator().inject_context(
                &span.context(),
                join_point
                    .get_mut_context()
                    .get_mut_request()
                    .get_mut_headers(),
            );
        };
        let future = async move {
            let start_time = get_now_date_time_as_millis();
            info!(message = "start handler");
            let context = join_point.proceed().await;
            info!(
                message = "end handler",
                elapsed = get_now_date_time_as_millis() - start_time,
            );
            context
        };
        let result = tokio::spawn(future.instrument(span)).await;
        let context = match result {
            Ok(context) => context,
            Err(error) => {
                error!(message = format!("{:?}", error));
                let mut context = FusenContext::new(
                    "unique_identifier".to_owned(),
                    Default::default(),
                    FusenRequest::new(None, Bytes::new()),
                    Default::default(),
                );
                *context.get_mut_response().get_mut_response() = Ok(Bytes::copy_from_slice(
                    b"{\"code\":\"9999\",\"message\":\"service error\"}",
                ));
                Ok(context)
            }
        };
        context
    }
}
