use crate::{
    Middleware, RpcArguments, RpcCategory, RpcError,
    service::ErasedDispatch,
    wire::{SERVICE_GROUP, SERVICE_VERSION},
};
use fusen_contract::{
    MethodDescriptor, ServiceDescriptor, SpringCloudParameterCardinality,
    SpringCloudParameterSource, WireProtocol,
};
use http::{HeaderMap, Method};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

#[derive(Clone)]
pub(crate) struct Route {
    pub protocol: WireProtocol,
    pub service: &'static ServiceDescriptor,
    pub method: &'static MethodDescriptor,
    pub dispatch: Arc<dyn ErasedDispatch>,
    pub head_middleware: Arc<[Arc<dyn Middleware>]>,
    pub middleware: Arc<[Arc<dyn Middleware>]>,
}

struct SpringRouteLeaf {
    route: Arc<Route>,
    parameter_names: Vec<String>,
}

#[derive(Default)]
struct SpringRouteTrie {
    literals: BTreeMap<String, SpringRouteTrie>,
    parameter: Option<Box<SpringRouteTrie>>,
    leaf: Option<SpringRouteLeaf>,
}

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
    spring: HashMap<Method, SpringRouteTrie>,
}

impl RouteTable {
    pub(crate) fn build(routes: Vec<Route>) -> Result<Self, String> {
        let mut fusen = HashMap::new();
        let mut spring = HashMap::new();
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
                    let segments = parse_template(mapping.path())?;
                    let method = mapping.method().clone();
                    let path = mapping.path().to_owned();
                    if !spring
                        .entry(method.clone())
                        .or_insert_with(SpringRouteTrie::default)
                        .insert(&segments, route)
                    {
                        return Err(format!(
                            "duplicate or ambiguous SpringCloudV1 route {} {}",
                            method, path
                        ));
                    }
                }
                _ => return Err("unsupported wire protocol in route table".into()),
            }
        }
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
        let trie = self.spring.get(method).ok_or_else(not_found)?;
        let mut parameter_values = Vec::new();
        let leaf = trie
            .match_segments(&request_segments, &mut parameter_values)
            .ok_or_else(not_found)?;
        debug_assert_eq!(leaf.parameter_names.len(), parameter_values.len());
        let path_arguments = leaf
            .parameter_names
            .iter()
            .cloned()
            .zip(parameter_values)
            .collect();
        Ok(MatchedRoute {
            route: leaf.route.clone(),
            path_arguments,
        })
    }
}

impl SpringRouteTrie {
    fn insert(&mut self, segments: &[Segment], route: Arc<Route>) -> bool {
        let mut node = self;
        let mut parameter_names = Vec::new();
        for segment in segments {
            node = match segment {
                Segment::Literal(value) => node.literals.entry(value.clone()).or_default(),
                Segment::Parameter(name) => {
                    parameter_names.push(name.clone());
                    node.parameter
                        .get_or_insert_with(|| Box::new(Self::default()))
                }
            };
        }
        if node.leaf.is_some() {
            return false;
        }
        node.leaf = Some(SpringRouteLeaf {
            route,
            parameter_names,
        });
        true
    }

    fn match_segments<'a>(
        &'a self,
        segments: &[String],
        parameter_values: &mut Vec<String>,
    ) -> Option<&'a SpringRouteLeaf> {
        let Some((segment, remaining)) = segments.split_first() else {
            return self.leaf.as_ref();
        };

        if let Some(literal) = self.literals.get(segment)
            && let Some(matched) = literal.match_segments(remaining, parameter_values)
        {
            return Some(matched);
        }

        let parameter = self.parameter.as_ref()?;
        parameter_values.push(segment.clone());
        let matched = parameter.match_segments(remaining, parameter_values);
        if matched.is_none() {
            parameter_values.pop();
        }
        matched
    }
}

impl MatchedRoute {
    pub(crate) fn spring_arguments(
        &self,
        query: Option<&str>,
        body: Option<Value>,
        max_query_pairs: usize,
    ) -> Result<RpcArguments, RpcError> {
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
        let mut arguments = RpcArguments::new();
        for parameter in mapping.parameters() {
            let value = match parameter.source() {
                SpringCloudParameterSource::Path => self
                    .path_arguments
                    .get(parameter.name())
                    .cloned()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
                SpringCloudParameterSource::Query => {
                    let values = query_values.remove(parameter.name()).unwrap_or_default();
                    match parameter.cardinality() {
                        SpringCloudParameterCardinality::Scalar => match values.as_slice() {
                            [] => Value::Null,
                            [value] => Value::String(value.clone()),
                            _ => {
                                return Err(RpcError::framework(
                                    RpcCategory::InvalidArgument,
                                    "duplicate_query_parameter",
                                    format!(
                                        "scalar query parameter {} must appear at most once",
                                        parameter.name()
                                    ),
                                ));
                            }
                        },
                        SpringCloudParameterCardinality::Repeated => {
                            Value::Array(values.into_iter().map(Value::String).collect())
                        }
                        _ => {
                            return Err(RpcError::framework(
                                RpcCategory::Unimplemented,
                                "unsupported_spring_parameter_cardinality",
                                "SpringCloudV1 parameter cardinality is not supported",
                            ));
                        }
                    }
                }
                SpringCloudParameterSource::BodyField => match body.as_ref() {
                    Some(Value::Object(fields)) => {
                        fields.get(parameter.name()).cloned().unwrap_or(Value::Null)
                    }
                    Some(_) => {
                        return Err(RpcError::framework(
                            RpcCategory::InvalidArgument,
                            "invalid_json_body",
                            "request body must be a JSON object",
                        ));
                    }
                    None => Value::Null,
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
            mapping.parameters().iter().any(|parameter| {
                matches!(
                    parameter.source(),
                    SpringCloudParameterSource::Body | SpringCloudParameterSource::BodyField
                )
            })
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

fn parse_template(path: &str) -> Result<Vec<Segment>, String> {
    path.trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            segment
                .strip_prefix('{')
                .and_then(|value| value.strip_suffix('}'))
                .map_or_else(
                    || {
                        urlencoding::decode(segment)
                            .map(|value| Segment::Literal(value.into_owned()))
                            .map_err(|_| {
                                "validated SpringCloudV1 route contains invalid percent encoding"
                                    .to_owned()
                            })
                    },
                    |name| Ok(Segment::Parameter(name.to_owned())),
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
    use crate::{MiddlewareFuture, service::ServerInvocation};
    use fusen_contract::{
        MethodId, ServiceSelector, SpringCloudMethod, SpringCloudParameter,
        SpringCloudParameterCardinality,
    };

    struct UnusedDispatch;

    impl ErasedDispatch for UnusedDispatch {
        fn call<'a>(&'a self, _invocation: ServerInvocation) -> MiddlewareFuture<'a> {
            Box::pin(async { unreachable!("route matching tests never dispatch") })
        }
    }

    fn spring_route(service_id: &str, method: Method, path: &str) -> Route {
        let parameters = parse_template(path)
            .expect("test route is valid")
            .into_iter()
            .filter_map(|segment| match segment {
                Segment::Literal(_) => None,
                Segment::Parameter(name) => Some(
                    SpringCloudParameter::new(
                        name,
                        SpringCloudParameterSource::Path,
                        SpringCloudParameterCardinality::Scalar,
                    )
                    .unwrap(),
                ),
            })
            .collect();
        spring_route_with_parameters(service_id, method, path, parameters)
    }

    fn spring_route_with_parameters(
        service_id: &str,
        method: Method,
        path: &str,
        parameters: Vec<SpringCloudParameter>,
    ) -> Route {
        let descriptor: &'static ServiceDescriptor = Box::leak(Box::new(
            ServiceDescriptor::new(
                ServiceSelector::new(service_id, None, None).unwrap(),
                vec![
                    MethodDescriptor::new(
                        MethodId::new(0),
                        service_id,
                        Some(SpringCloudMethod::new(method, path, parameters).unwrap()),
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        ));
        Route {
            protocol: WireProtocol::SpringCloudV1,
            service: descriptor,
            method: &descriptor.methods()[0],
            dispatch: Arc::new(UnusedDispatch),
            head_middleware: Arc::from(Vec::<Arc<dyn Middleware>>::new()),
            middleware: Arc::from(Vec::<Arc<dyn Middleware>>::new()),
        }
    }

    #[test]
    fn spring_routes_use_per_segment_precedence_independent_of_insertion_order() {
        for parameter_first in [false, true] {
            let literal_prefix = spring_route(
                "literal-prefix",
                Method::GET,
                "/files/special/{kind}/{value}",
            );
            let more_total_literals = spring_route(
                "more-total-literals",
                Method::GET,
                "/files/{id}/details/static",
            );
            let routes = if parameter_first {
                vec![more_total_literals, literal_prefix]
            } else {
                vec![literal_prefix, more_total_literals]
            };
            let table = RouteTable::build(routes).unwrap();

            let matched = table
                .match_spring(&Method::GET, "/files/special/details/static")
                .unwrap();
            assert_eq!(matched.route.method.fusen_identity(), "literal-prefix");
            assert_eq!(
                matched.path_arguments.get("kind").map(String::as_str),
                Some("details")
            );
            assert_eq!(
                matched.path_arguments.get("value").map(String::as_str),
                Some("static")
            );
        }
    }

    #[test]
    fn spring_routes_fall_back_to_a_parameter_after_a_literal_dead_end() {
        let table = RouteTable::build(vec![
            spring_route("literal-dead-end", Method::GET, "/catalog/special/details"),
            spring_route("parameter-fallback", Method::GET, "/catalog/{id}/summary"),
        ])
        .unwrap();

        let matched = table
            .match_spring(&Method::GET, "/catalog/special/summary")
            .unwrap();
        assert_eq!(matched.route.method.fusen_identity(), "parameter-fallback");
        assert_eq!(
            matched.path_arguments.get("id").map(String::as_str),
            Some("special")
        );
    }

    #[test]
    fn spring_routes_match_decoded_literal_and_parameter_segments() {
        let table = RouteTable::build(vec![
            spring_route("encoded-literal", Method::GET, "/encoded/special"),
            spring_route("unicode-literal", Method::GET, "/encoded/%E7%94%A8"),
            spring_route("encoded-parameter", Method::GET, "/encoded/{value}"),
        ])
        .unwrap();

        let literal = table
            .match_spring(&Method::GET, "/encoded/%73pecial")
            .unwrap();
        assert_eq!(literal.route.method.fusen_identity(), "encoded-literal");

        let unicode = table
            .match_spring(&Method::GET, "/encoded/%E7%94%A8")
            .unwrap();
        assert_eq!(unicode.route.method.fusen_identity(), "unicode-literal");

        let parameter = table.match_spring(&Method::GET, "/encoded/a%2Fb").unwrap();
        assert_eq!(
            parameter.path_arguments.get("value").map(String::as_str),
            Some("a/b")
        );
    }

    #[test]
    fn spring_routes_reject_equivalent_dynamic_shapes_across_services() {
        let result = RouteTable::build(vec![
            spring_route("route-by-id", Method::GET, "/users/{id}"),
            spring_route("route-by-name", Method::GET, "/users/{name}"),
        ]);

        let error = match result {
            Ok(_) => panic!("equivalent dynamic route shapes must be rejected"),
            Err(error) => error,
        };
        assert!(error.contains("duplicate or ambiguous SpringCloudV1 route GET /users/{name}"));
    }

    #[test]
    fn spring_route_tries_are_isolated_by_http_method() {
        let table = RouteTable::build(vec![
            spring_route("get-route", Method::GET, "/method/{id}"),
            spring_route("post-route", Method::POST, "/method/{name}"),
        ])
        .unwrap();

        let get = table.match_spring(&Method::GET, "/method/value").unwrap();
        let post = table.match_spring(&Method::POST, "/method/value").unwrap();
        assert_eq!(get.route.method.fusen_identity(), "get-route");
        assert_eq!(post.route.method.fusen_identity(), "post-route");
    }

    #[test]
    fn fusen_routes_remain_exact() {
        let mut route = spring_route("fusen-exact", Method::GET, "/unused");
        route.protocol = WireProtocol::FusenV1;
        let table = RouteTable::build(vec![route]).unwrap();

        let matched = table
            .match_fusen("/_fusen/v1/fusen-exact/fusen-exact", &HeaderMap::new())
            .unwrap();
        assert_eq!(matched.route.method.fusen_identity(), "fusen-exact");
        assert!(
            table
                .match_fusen("/_fusen/v1/fusen-exact/missing", &HeaderMap::new())
                .is_err()
        );
    }

    #[test]
    fn spring_query_cardinality_has_stable_missing_single_and_duplicate_semantics() {
        let route = spring_route_with_parameters(
            "query-cardinality",
            Method::GET,
            "/query",
            vec![
                SpringCloudParameter::new(
                    "enabled",
                    SpringCloudParameterSource::Query,
                    SpringCloudParameterCardinality::Scalar,
                )
                .unwrap(),
                SpringCloudParameter::new(
                    "tag",
                    SpringCloudParameterSource::Query,
                    SpringCloudParameterCardinality::Repeated,
                )
                .unwrap(),
            ],
        );
        let table = RouteTable::build(vec![route]).unwrap();
        let matched = table.match_spring(&Method::GET, "/query").unwrap();

        let missing = matched.spring_arguments(None, None, 8).unwrap();
        assert_eq!(missing.get("enabled"), Some(&Value::Null));
        assert_eq!(missing.get("tag"), Some(&Value::Array(Vec::new())));

        let present = matched
            .spring_arguments(Some("enabled=true&tag=one&tag=two"), None, 8)
            .unwrap();
        assert_eq!(present.get("enabled"), Some(&Value::String("true".into())));
        assert_eq!(present.get("tag"), Some(&serde_json::json!(["one", "two"])));

        let duplicate = matched
            .spring_arguments(Some("enabled=true&enabled=false"), None, 8)
            .unwrap_err();
        assert_eq!(duplicate.category(), RpcCategory::InvalidArgument);
        assert_eq!(duplicate.code().as_str(), "duplicate_query_parameter");
    }

    #[test]
    fn spring_body_fields_are_extracted_from_one_json_object() {
        let parameters = ["name", "audit"]
            .into_iter()
            .map(|name| {
                SpringCloudParameter::new(
                    name,
                    SpringCloudParameterSource::BodyField,
                    SpringCloudParameterCardinality::Scalar,
                )
                .unwrap()
            })
            .collect();
        let route = spring_route_with_parameters("body-fields", Method::POST, "/users", parameters);
        let table = RouteTable::build(vec![route]).unwrap();
        let matched = table.match_spring(&Method::POST, "/users").unwrap();
        assert!(matched.spring_has_body());

        let arguments = matched
            .spring_arguments(
                None,
                Some(serde_json::json!({"name": "Ada", "audit": true})),
                0,
            )
            .unwrap();
        assert_eq!(arguments.get("name"), Some(&serde_json::json!("Ada")));
        assert_eq!(arguments.get("audit"), Some(&serde_json::json!(true)));

        let error = matched
            .spring_arguments(None, Some(serde_json::json!(["not", "an", "object"])), 0)
            .unwrap_err();
        assert_eq!(error.code().as_str(), "invalid_json_body");
    }

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
