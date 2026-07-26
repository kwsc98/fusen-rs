use crate::{
    Arguments, RpcCategory, RpcError,
    middleware::MiddlewareDyn,
    service::ErasedDispatch,
    wire::{SERVICE_GROUP, SERVICE_VERSION},
};
use fusen_contract::{
    MethodDescriptor, ServiceDescriptor, SpringCloudParameterSource, WireProtocol,
};
use http::{HeaderMap, Method};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

#[derive(Clone)]
pub(crate) struct Route {
    pub protocol: WireProtocol,
    pub service: &'static ServiceDescriptor,
    pub method: &'static MethodDescriptor,
    pub dispatch: Arc<dyn ErasedDispatch>,
    pub middleware: Arc<[Arc<dyn MiddlewareDyn>]>,
}

#[derive(Clone)]
struct SpringRoute {
    method: Method,
    segments: Vec<Segment>,
    route: Arc<Route>,
}

#[derive(Clone)]
enum Segment {
    Literal(String),
    Parameter(String),
}

pub(crate) struct MatchedRoute {
    pub route: Arc<Route>,
    pub path_arguments: HashMap<String, String>,
}

pub(crate) struct RouteTable {
    fusen: HashMap<(String, Option<String>, Option<String>), Arc<Route>>,
    spring: Vec<SpringRoute>,
}

impl RouteTable {
    pub(crate) fn build(routes: Vec<Route>) -> Result<Self, String> {
        let mut fusen = HashMap::new();
        let mut spring = Vec::new();
        let mut spring_keys = HashSet::new();
        for route in routes {
            let route = Arc::new(route);
            match route.protocol {
                WireProtocol::FusenV1 => {
                    let path = format!(
                        "/_fusen/v1/{}/{}",
                        route.service.selector().service_id(),
                        route.method.fusen_identity()
                    );
                    let key = (
                        path,
                        route.service.selector().group().map(str::to_owned),
                        route.service.selector().version().map(str::to_owned),
                    );
                    if fusen.insert(key, route).is_some() {
                        return Err("duplicate FusenV1 service/method route".into());
                    }
                }
                WireProtocol::SpringCloudV1 => {
                    let mapping = route.method.spring_cloud().ok_or_else(|| {
                        format!(
                            "service {} method {} has no SpringCloudV1 mapping",
                            route.service.identity(),
                            route.method.fusen_identity()
                        )
                    })?;
                    if mapping.path().starts_with("/_fusen/v1/") {
                        return Err(
                            "SpringCloudV1 routes may not use the reserved /_fusen/v1 prefix"
                                .into(),
                        );
                    }
                    let segments = parse_template(mapping.path());
                    let normalized = segments
                        .iter()
                        .map(|segment| match segment {
                            Segment::Literal(value) => value.as_str(),
                            Segment::Parameter(_) => "{}",
                        })
                        .collect::<Vec<_>>()
                        .join("/");
                    if !spring_keys.insert((mapping.method().clone(), normalized)) {
                        return Err(format!(
                            "duplicate or ambiguous SpringCloudV1 route {} {}",
                            mapping.method(),
                            mapping.path()
                        ));
                    }
                    spring.push(SpringRoute {
                        method: mapping.method().clone(),
                        segments,
                        route,
                    });
                }
                _ => return Err("unsupported wire protocol in route table".into()),
            }
        }
        spring.sort_by(|left, right| {
            left.method
                .as_str()
                .cmp(right.method.as_str())
                .then_with(|| {
                    let left = left
                        .segments
                        .iter()
                        .filter(|segment| matches!(segment, Segment::Literal(_)))
                        .count();
                    let right = right
                        .segments
                        .iter()
                        .filter(|segment| matches!(segment, Segment::Literal(_)))
                        .count();
                    right.cmp(&left)
                })
        });
        Ok(Self { fusen, spring })
    }

    pub(crate) fn match_fusen(
        &self,
        path: &str,
        headers: &HeaderMap,
    ) -> Result<MatchedRoute, RpcError> {
        let group = single_header(headers, &SERVICE_GROUP)?;
        let version = single_header(headers, &SERVICE_VERSION)?;
        let key = (path.to_owned(), group, version);
        let route = self.fusen.get(&key).cloned().ok_or_else(not_found)?;
        Ok(MatchedRoute {
            route,
            path_arguments: HashMap::new(),
        })
    }

    pub(crate) fn match_spring(
        &self,
        method: &Method,
        path: &str,
    ) -> Result<MatchedRoute, RpcError> {
        let request_segments = split_path(path)?;
        for candidate in &self.spring {
            if candidate.method != *method || candidate.segments.len() != request_segments.len() {
                continue;
            }
            let mut parameters = HashMap::new();
            let mut matched = true;
            for (expected, actual) in candidate.segments.iter().zip(&request_segments) {
                match expected {
                    Segment::Literal(value) if value == actual => {}
                    Segment::Literal(_) => {
                        matched = false;
                        break;
                    }
                    Segment::Parameter(name) => {
                        parameters.insert(name.clone(), actual.clone());
                    }
                }
            }
            if matched {
                return Ok(MatchedRoute {
                    route: candidate.route.clone(),
                    path_arguments: parameters,
                });
            }
        }
        Err(not_found())
    }
}

impl MatchedRoute {
    pub(crate) fn spring_arguments(
        &self,
        query: Option<&str>,
        body: Option<Value>,
        max_query_pairs: usize,
    ) -> Result<Arguments, RpcError> {
        let mapping = self
            .route
            .method
            .spring_cloud()
            .expect("Spring route always has Spring metadata");
        let mut query_values: HashMap<String, Vec<String>> = HashMap::new();
        for (index, (name, value)) in
            url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()).enumerate()
        {
            if index >= max_query_pairs {
                return Err(RpcError::framework(
                    RpcCategory::InvalidArgument,
                    "too_many_query_pairs",
                    "request query contains too many pairs",
                ));
            }
            query_values
                .entry(name.into_owned())
                .or_default()
                .push(value.into_owned());
        }
        let mut arguments = Arguments::new();
        for parameter in mapping.parameters() {
            let value = match parameter.source() {
                SpringCloudParameterSource::Path => self
                    .path_arguments
                    .get(parameter.name())
                    .cloned()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
                SpringCloudParameterSource::Query => match query_values.remove(parameter.name()) {
                    None => Value::Null,
                    Some(mut values) if values.len() == 1 => Value::String(values.remove(0)),
                    Some(values) => Value::Array(values.into_iter().map(Value::String).collect()),
                },
                SpringCloudParameterSource::Body => body.clone().unwrap_or(Value::Null),
                _ => {
                    return Err(RpcError::framework(
                        RpcCategory::Unimplemented,
                        "unsupported_spring_parameter_source",
                        "SpringCloudV1 parameter source is not supported",
                    ));
                }
            };
            arguments.insert(parameter.name().to_owned(), value);
        }
        Ok(arguments)
    }

    pub(crate) fn spring_has_body(&self) -> bool {
        self.route.method.spring_cloud().is_some_and(|mapping| {
            mapping
                .parameters()
                .iter()
                .any(|parameter| parameter.source() == SpringCloudParameterSource::Body)
        })
    }
}

pub(crate) fn validate_query_pairs(
    query: Option<&str>,
    max_query_pairs: usize,
) -> Result<(), RpcError> {
    if url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .take(max_query_pairs.saturating_add(1))
        .count()
        > max_query_pairs
    {
        Err(RpcError::framework(
            RpcCategory::InvalidArgument,
            "too_many_query_pairs",
            "request query contains too many pairs",
        ))
    } else {
        Ok(())
    }
}

fn parse_template(path: &str) -> Vec<Segment> {
    path.trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            segment
                .strip_prefix('{')
                .and_then(|value| value.strip_suffix('}'))
                .map_or_else(
                    || Segment::Literal(segment.to_owned()),
                    |name| Segment::Parameter(name.to_owned()),
                )
        })
        .collect()
}

fn split_path(path: &str) -> Result<Vec<String>, RpcError> {
    if path != "/" && (path.ends_with('/') || path.contains("//")) {
        return Err(not_found());
    }
    path.trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            urlencoding::decode(segment)
                .map(|value| value.into_owned())
                .map_err(|_| {
                    RpcError::framework(
                        RpcCategory::InvalidArgument,
                        "invalid_path_encoding",
                        "request path contains invalid percent encoding",
                    )
                })
        })
        .collect()
}

fn single_header(headers: &HeaderMap, name: &http::HeaderName) -> Result<Option<String>, RpcError> {
    let mut values = headers.get_all(name).iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(RpcError::framework(
            RpcCategory::InvalidArgument,
            "duplicate_service_header",
            format!("service identity header {name} must appear at most once"),
        ));
    }
    value
        .map(|value| {
            value.to_str().map(str::to_owned).map_err(|_| {
                RpcError::framework(
                    RpcCategory::InvalidArgument,
                    "invalid_service_header",
                    format!("service identity header {name} is invalid"),
                )
            })
        })
        .transpose()
}

fn not_found() -> RpcError {
    RpcError::framework(
        RpcCategory::NotFound,
        "route_not_found",
        "RPC route was not found",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_pair_limit_is_checked_without_decoding_a_body() {
        assert!(validate_query_pairs(None, 1).is_ok());
        assert!(validate_query_pairs(Some("first=1&second=2"), 2).is_ok());
        let error = validate_query_pairs(Some("first=1&second=2&third=3"), 2).unwrap_err();
        assert_eq!(error.code().as_str(), "too_many_query_pairs");
    }

    #[test]
    fn incoming_paths_do_not_normalize_empty_segments() {
        assert!(split_path("/").unwrap().is_empty());
        assert_eq!(split_path("/users/one").unwrap(), ["users", "one"]);
        for path in ["//users/one", "/users//one", "/users/one/"] {
            assert_eq!(
                split_path(path).unwrap_err().code().as_str(),
                "route_not_found"
            );
        }
    }
}
