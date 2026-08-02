use crate::{Arguments, Error, ErrorCategory, Interceptor, service::ErasedDispatch};
use fusen_contract::{
    HttpParameterCardinality, HttpParameterSource, MethodDescriptor, ServiceDescriptor,
};
use http::{HeaderMap, Method, header::COOKIE};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

#[derive(Clone)]
pub(crate) struct Route {
    pub service: &'static ServiceDescriptor,
    pub method: &'static MethodDescriptor,
    pub dispatch: Arc<dyn ErasedDispatch>,
    pub head_interceptor: Arc<[Arc<dyn Interceptor>]>,
    pub interceptor: Arc<[Arc<dyn Interceptor>]>,
}

struct HttpRouteLeaf {
    route: Arc<Route>,
    parameter_names: Vec<String>,
}

#[derive(Default)]
struct HttpRouteTrie {
    literals: BTreeMap<String, HttpRouteTrie>,
    parameter: Option<Box<HttpRouteTrie>>,
    leaf: Option<HttpRouteLeaf>,
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
    http: HashMap<Method, HttpRouteTrie>,
}

impl RouteTable {
    pub(crate) fn build(routes: Vec<Route>) -> Result<Self, String> {
        let mut http = HashMap::new();
        for route in routes {
            let route = Arc::new(route);
            let mapping = route.method.http_operation();
            let segments = parse_template(mapping.path())?;
            let method = mapping.method().clone();
            let path = mapping.path().to_owned();
            if !http
                .entry(method.clone())
                .or_insert_with(HttpRouteTrie::default)
                .insert(&segments, route)
            {
                return Err(format!(
                    "duplicate or ambiguous HTTP route {} {}",
                    method, path
                ));
            }
        }
        Ok(Self { http })
    }

    pub(crate) fn match_http(&self, method: &Method, path: &str) -> Result<MatchedRoute, Error> {
        let request_segments = split_path(path)?;
        let trie = self.http.get(method).ok_or_else(not_found)?;
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

impl HttpRouteTrie {
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
        node.leaf = Some(HttpRouteLeaf {
            route,
            parameter_names,
        });
        true
    }

    fn match_segments<'a>(
        &'a self,
        segments: &[String],
        parameter_values: &mut Vec<String>,
    ) -> Option<&'a HttpRouteLeaf> {
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
    pub(crate) fn http_arguments(
        &self,
        query: Option<&str>,
        headers: &HeaderMap,
        body: Option<Value>,
        max_query_pairs: usize,
    ) -> Result<Arguments, Error> {
        let mapping = self.route.method.http_operation();
        let mut query_values: HashMap<String, Vec<String>> = HashMap::new();
        for (index, (name, value)) in
            url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()).enumerate()
        {
            if index >= max_query_pairs {
                return Err(Error::framework(
                    ErrorCategory::InvalidArgument,
                    "too_many_query_pairs",
                    "request query contains too many pairs",
                ));
            }
            query_values
                .entry(name.into_owned())
                .or_default()
                .push(value.into_owned());
        }
        let cookies = parse_cookies(headers)?;
        let explicit_query = mapping
            .parameters()
            .iter()
            .filter(|parameter| parameter.source() == HttpParameterSource::Query)
            .map(|parameter| parameter.name())
            .collect::<std::collections::HashSet<_>>();
        let explicit_headers = mapping
            .parameters()
            .iter()
            .filter(|parameter| parameter.source() == HttpParameterSource::Header)
            .map(|parameter| parameter.name().to_ascii_lowercase())
            .collect::<std::collections::HashSet<_>>();
        let mut arguments = Arguments::new();
        for parameter in mapping.parameters() {
            let value = match parameter.source() {
                HttpParameterSource::Path => self
                    .path_arguments
                    .get(parameter.name())
                    .cloned()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
                HttpParameterSource::Query => {
                    let values = query_values.remove(parameter.name()).unwrap_or_default();
                    match parameter.cardinality() {
                        HttpParameterCardinality::Scalar => match values.as_slice() {
                            [] => Value::Null,
                            [value] => Value::String(value.clone()),
                            _ => {
                                return Err(Error::framework(
                                    ErrorCategory::InvalidArgument,
                                    "duplicate_query_parameter",
                                    format!(
                                        "scalar query parameter {} must appear at most once",
                                        parameter.name()
                                    ),
                                ));
                            }
                        },
                        HttpParameterCardinality::Repeated => {
                            Value::Array(values.into_iter().map(Value::String).collect())
                        }
                        _ => {
                            return Err(Error::framework(
                                ErrorCategory::Unimplemented,
                                "unsupported_http_parameter_cardinality",
                                "HTTP parameter cardinality is not supported",
                            ));
                        }
                    }
                }
                HttpParameterSource::Header => header_value(headers, parameter.name())?,
                HttpParameterSource::Cookie => cookies
                    .get(parameter.name())
                    .cloned()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
                HttpParameterSource::BodyField => match body.as_ref() {
                    Some(Value::Object(fields)) => {
                        fields.get(parameter.name()).cloned().unwrap_or(Value::Null)
                    }
                    Some(_) => {
                        return Err(Error::framework(
                            ErrorCategory::InvalidArgument,
                            "invalid_json_body",
                            "request body must be a JSON object",
                        ));
                    }
                    None => Value::Null,
                },
                HttpParameterSource::Body => body.clone().unwrap_or(Value::Null),
                HttpParameterSource::QueryMap => {
                    Value::Object(query_map(&query_values, &explicit_query))
                }
                HttpParameterSource::HeaderMap => {
                    Value::Object(header_map(headers, &explicit_headers)?)
                }
                _ => {
                    return Err(Error::framework(
                        ErrorCategory::Unimplemented,
                        "unsupported_http_parameter_source",
                        "HTTP parameter source is not supported",
                    ));
                }
            };
            arguments.insert(parameter.name().to_owned(), value);
        }
        Ok(arguments)
    }

    pub(crate) fn has_body(&self) -> bool {
        self.route
            .method
            .http_operation()
            .parameters()
            .iter()
            .any(|parameter| {
                matches!(
                    parameter.source(),
                    HttpParameterSource::Body | HttpParameterSource::BodyField
                )
            })
    }
}

fn header_value(headers: &HeaderMap, name: &str) -> Result<Value, Error> {
    let values = headers
        .get_all(name)
        .iter()
        .map(|value| {
            value.to_str().map(str::to_owned).map_err(|_| {
                Error::framework(
                    ErrorCategory::InvalidArgument,
                    "invalid_header_parameter",
                    format!("HTTP header parameter {name} is not valid text"),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    match values.as_slice() {
        [] => Ok(Value::Null),
        [value] => Ok(Value::String(value.clone())),
        _ => Err(Error::framework(
            ErrorCategory::InvalidArgument,
            "duplicate_header_parameter",
            format!("scalar HTTP header parameter {name} must appear at most once"),
        )),
    }
}

fn parse_cookies(headers: &HeaderMap) -> Result<HashMap<String, String>, Error> {
    let mut cookies = HashMap::new();
    for header in headers.get_all(COOKIE) {
        let value = header.to_str().map_err(|_| {
            Error::framework(
                ErrorCategory::InvalidArgument,
                "invalid_cookie_header",
                "Cookie header is not valid text",
            )
        })?;
        for pair in value.split(';') {
            let Some((name, value)) = pair.trim().split_once('=') else {
                return Err(Error::framework(
                    ErrorCategory::InvalidArgument,
                    "invalid_cookie_header",
                    "Cookie header contains an invalid pair",
                ));
            };
            if cookies.insert(name.to_owned(), value.to_owned()).is_some() {
                return Err(Error::framework(
                    ErrorCategory::InvalidArgument,
                    "duplicate_cookie_parameter",
                    format!("cookie {name} must appear at most once"),
                ));
            }
        }
    }
    Ok(cookies)
}

fn query_map(
    values: &HashMap<String, Vec<String>>,
    explicit: &std::collections::HashSet<&str>,
) -> Map<String, Value> {
    values
        .iter()
        .filter(|(name, _)| !explicit.contains(name.as_str()))
        .map(|(name, values)| {
            let value = match values.as_slice() {
                [value] => Value::String(value.clone()),
                values => Value::Array(values.iter().cloned().map(Value::String).collect()),
            };
            (name.clone(), value)
        })
        .collect()
}

fn header_map(
    headers: &HeaderMap,
    explicit: &std::collections::HashSet<String>,
) -> Result<Map<String, Value>, Error> {
    let mut values: HashMap<String, Vec<String>> = HashMap::new();
    for (name, value) in headers {
        if explicit.contains(name.as_str()) || name == COOKIE {
            continue;
        }
        let value = value.to_str().map_err(|_| {
            Error::framework(
                ErrorCategory::InvalidArgument,
                "invalid_header_map",
                "header_map contains a non-text header value",
            )
        })?;
        values
            .entry(name.as_str().to_owned())
            .or_default()
            .push(value.to_owned());
    }
    Ok(values
        .into_iter()
        .map(|(name, values)| {
            let value = match values.as_slice() {
                [value] => Value::String(value.clone()),
                values => Value::Array(values.iter().cloned().map(Value::String).collect()),
            };
            (name, value)
        })
        .collect())
}

pub(crate) fn validate_query_pairs(
    query: Option<&str>,
    max_query_pairs: usize,
) -> Result<(), Error> {
    if url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .take(max_query_pairs.saturating_add(1))
        .count()
        > max_query_pairs
    {
        Err(Error::framework(
            ErrorCategory::InvalidArgument,
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
                                "validated HTTP route contains invalid percent encoding".to_owned()
                            })
                    },
                    |name| Ok(Segment::Parameter(name.to_owned())),
                )
        })
        .collect()
}

fn split_path(path: &str) -> Result<Vec<String>, Error> {
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
                    Error::framework(
                        ErrorCategory::InvalidArgument,
                        "invalid_path_encoding",
                        "request path contains invalid percent encoding",
                    )
                })
        })
        .collect()
}

fn not_found() -> Error {
    Error::framework(
        ErrorCategory::NotFound,
        "route_not_found",
        "service invocation route was not found",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InterceptorFuture, service::ServerInvocation};
    use fusen_contract::{
        HttpOperation, HttpParameter, HttpParameterCardinality, HttpParameterSource, MethodId,
        ServiceSelector,
    };

    struct UnusedDispatch;

    impl ErasedDispatch for UnusedDispatch {
        fn call<'a>(&'a self, _invocation: ServerInvocation) -> InterceptorFuture<'a> {
            Box::pin(async { unreachable!("route matching tests never dispatch") })
        }
    }

    fn http_route(service_id: &str, method: Method, path: &str) -> Route {
        let parameters = parse_template(path)
            .expect("test route is valid")
            .into_iter()
            .filter_map(|segment| match segment {
                Segment::Literal(_) => None,
                Segment::Parameter(name) => Some(
                    HttpParameter::new(
                        name,
                        HttpParameterSource::Path,
                        HttpParameterCardinality::Scalar,
                    )
                    .unwrap(),
                ),
            })
            .collect();
        http_route_with_parameters(service_id, method, path, parameters)
    }

    fn http_route_with_parameters(
        service_id: &str,
        method: Method,
        path: &str,
        parameters: Vec<HttpParameter>,
    ) -> Route {
        let descriptor: &'static ServiceDescriptor = Box::leak(Box::new(
            ServiceDescriptor::new(
                ServiceSelector::new(service_id, None, None).unwrap(),
                vec![
                    MethodDescriptor::new(
                        MethodId::new(0),
                        service_id,
                        HttpOperation::new(
                            method,
                            path,
                            parameters,
                            "application/json",
                            "application/json",
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        ));
        Route {
            service: descriptor,
            method: &descriptor.methods()[0],
            dispatch: Arc::new(UnusedDispatch),
            head_interceptor: Arc::from(Vec::<Arc<dyn Interceptor>>::new()),
            interceptor: Arc::from(Vec::<Arc<dyn Interceptor>>::new()),
        }
    }

    #[test]
    fn http_routes_use_per_segment_precedence_independent_of_insertion_order() {
        for parameter_first in [false, true] {
            let literal_prefix = http_route(
                "literal-prefix",
                Method::GET,
                "/files/special/{kind}/{value}",
            );
            let more_total_literals = http_route(
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
                .match_http(&Method::GET, "/files/special/details/static")
                .unwrap();
            assert_eq!(matched.route.method.invocation_name(), "literal-prefix");
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
    fn http_routes_fall_back_to_a_parameter_after_a_literal_dead_end() {
        let table = RouteTable::build(vec![
            http_route("literal-dead-end", Method::GET, "/catalog/special/details"),
            http_route("parameter-fallback", Method::GET, "/catalog/{id}/summary"),
        ])
        .unwrap();

        let matched = table
            .match_http(&Method::GET, "/catalog/special/summary")
            .unwrap();
        assert_eq!(matched.route.method.invocation_name(), "parameter-fallback");
        assert_eq!(
            matched.path_arguments.get("id").map(String::as_str),
            Some("special")
        );
    }

    #[test]
    fn http_routes_match_decoded_literal_and_parameter_segments() {
        let table = RouteTable::build(vec![
            http_route("encoded-literal", Method::GET, "/encoded/special"),
            http_route("unicode-literal", Method::GET, "/encoded/%E7%94%A8"),
            http_route("encoded-parameter", Method::GET, "/encoded/{value}"),
        ])
        .unwrap();

        let literal = table
            .match_http(&Method::GET, "/encoded/%73pecial")
            .unwrap();
        assert_eq!(literal.route.method.invocation_name(), "encoded-literal");

        let unicode = table
            .match_http(&Method::GET, "/encoded/%E7%94%A8")
            .unwrap();
        assert_eq!(unicode.route.method.invocation_name(), "unicode-literal");

        let parameter = table.match_http(&Method::GET, "/encoded/a%2Fb").unwrap();
        assert_eq!(
            parameter.path_arguments.get("value").map(String::as_str),
            Some("a/b")
        );
    }

    #[test]
    fn http_routes_reject_equivalent_dynamic_shapes_across_services() {
        let result = RouteTable::build(vec![
            http_route("route-by-id", Method::GET, "/users/{id}"),
            http_route("route-by-name", Method::GET, "/users/{name}"),
        ]);

        let error = match result {
            Ok(_) => panic!("equivalent dynamic route shapes must be rejected"),
            Err(error) => error,
        };
        assert!(error.contains("duplicate or ambiguous HTTP route GET /users/{name}"));
    }

    #[test]
    fn http_route_tries_are_isolated_by_http_method() {
        let table = RouteTable::build(vec![
            http_route("get-route", Method::GET, "/method/{id}"),
            http_route("post-route", Method::POST, "/method/{name}"),
        ])
        .unwrap();

        let get = table.match_http(&Method::GET, "/method/value").unwrap();
        let post = table.match_http(&Method::POST, "/method/value").unwrap();
        assert_eq!(get.route.method.invocation_name(), "get-route");
        assert_eq!(post.route.method.invocation_name(), "post-route");
    }

    #[test]
    fn http_query_cardinality_has_stable_missing_single_and_duplicate_semantics() {
        let route = http_route_with_parameters(
            "query-cardinality",
            Method::GET,
            "/query",
            vec![
                HttpParameter::new(
                    "enabled",
                    HttpParameterSource::Query,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
                HttpParameter::new(
                    "tag",
                    HttpParameterSource::Query,
                    HttpParameterCardinality::Repeated,
                )
                .unwrap(),
            ],
        );
        let table = RouteTable::build(vec![route]).unwrap();
        let matched = table.match_http(&Method::GET, "/query").unwrap();

        let missing = matched
            .http_arguments(None, &HeaderMap::new(), None, 8)
            .unwrap();
        assert_eq!(missing.get("enabled"), Some(&Value::Null));
        assert_eq!(missing.get("tag"), Some(&Value::Array(Vec::new())));

        let present = matched
            .http_arguments(
                Some("enabled=true&tag=one&tag=two"),
                &HeaderMap::new(),
                None,
                8,
            )
            .unwrap();
        assert_eq!(present.get("enabled"), Some(&Value::String("true".into())));
        assert_eq!(present.get("tag"), Some(&serde_json::json!(["one", "two"])));

        let duplicate = matched
            .http_arguments(
                Some("enabled=true&enabled=false"),
                &HeaderMap::new(),
                None,
                8,
            )
            .unwrap_err();
        assert_eq!(duplicate.category(), ErrorCategory::InvalidArgument);
        assert_eq!(duplicate.code().as_str(), "duplicate_query_parameter");
    }

    #[test]
    fn parameter_maps_restore_unclaimed_query_and_header_values() {
        let route = http_route_with_parameters(
            "parameter-maps",
            Method::GET,
            "/maps",
            vec![
                HttpParameter::new(
                    "page",
                    HttpParameterSource::Query,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
                HttpParameter::new(
                    "query",
                    HttpParameterSource::QueryMap,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
                HttpParameter::new(
                    "x-tenant",
                    HttpParameterSource::Header,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
                HttpParameter::new(
                    "headers",
                    HttpParameterSource::HeaderMap,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
            ],
        );
        let table = RouteTable::build(vec![route]).unwrap();
        let matched = table.match_http(&Method::GET, "/maps").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-tenant", "acme".parse().unwrap());
        headers.append("x-feature", "one".parse().unwrap());
        headers.append("x-feature", "two".parse().unwrap());
        headers.insert(COOKIE, "session=private".parse().unwrap());

        let arguments = matched
            .http_arguments(Some("page=1&tag=one&tag=two"), &headers, None, 8)
            .unwrap();
        assert_eq!(arguments.get("page"), Some(&serde_json::json!("1")));
        assert_eq!(
            arguments.get("query"),
            Some(&serde_json::json!({"tag": ["one", "two"]}))
        );
        assert_eq!(arguments.get("x-tenant"), Some(&serde_json::json!("acme")));
        assert_eq!(
            arguments.get("headers"),
            Some(&serde_json::json!({"x-feature": ["one", "two"]}))
        );
    }

    #[test]
    fn http_body_fields_are_extracted_from_one_json_object() {
        let parameters = ["name", "audit"]
            .into_iter()
            .map(|name| {
                HttpParameter::new(
                    name,
                    HttpParameterSource::BodyField,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap()
            })
            .collect();
        let route = http_route_with_parameters("body-fields", Method::POST, "/users", parameters);
        let table = RouteTable::build(vec![route]).unwrap();
        let matched = table.match_http(&Method::POST, "/users").unwrap();
        assert!(matched.has_body());

        let arguments = matched
            .http_arguments(
                None,
                &HeaderMap::new(),
                Some(serde_json::json!({"name": "Ada", "audit": true})),
                0,
            )
            .unwrap();
        assert_eq!(arguments.get("name"), Some(&serde_json::json!("Ada")));
        assert_eq!(arguments.get("audit"), Some(&serde_json::json!(true)));

        let error = matched
            .http_arguments(
                None,
                &HeaderMap::new(),
                Some(serde_json::json!(["not", "an", "object"])),
                0,
            )
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
