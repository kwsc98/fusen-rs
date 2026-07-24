use crate::error::FusenError;
use fusen_contract::{MethodDescriptor, ParameterSource, ServiceEndpoint, WireProtocol};
use http::{HeaderMap, Method};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};

#[derive(Debug, Clone)]
/// Internal HTTP method and route template for one RPC request.
pub struct Path {
    /// HTTP method used by the request.
    pub method: Method,
    /// Relative service route or decoded request path.
    pub path: String,
}

pub type QueryParameters = BTreeMap<String, Vec<String>>;

#[derive(Debug)]
pub enum ArgumentValue {
    Text(String),
    Json(Value),
}

impl ArgumentValue {
    pub fn deserialize<T: DeserializeOwned>(self) -> Result<T, FusenError> {
        match self {
            Self::Json(value) => serde_json::from_value(value)
                .map_err(|error| FusenError::InvalidRequest(error.to_string())),
            Self::Text(value) => serde_json::from_value(Value::String(value.clone()))
                .or_else(|_| serde_json::from_str(&value))
                .map_err(|error| FusenError::InvalidRequest(error.to_string())),
        }
    }
}

#[derive(Debug)]
/// Internal mutable request representation shared by middleware and transports.
pub struct FusenRequest {
    /// Wire protocol selected for this request.
    pub protocol: WireProtocol,
    /// HTTP method and route.
    pub path: Path,
    /// Endpoint selected by the client cluster, when available.
    pub endpoint: Option<ServiceEndpoint>,
    /// Path arguments keyed by descriptor parameter name.
    pub path_parameters: HashMap<String, String>,
    /// Query arguments keyed by descriptor parameter name.
    pub query_parameters: QueryParameters,
    /// Request headers.
    pub headers: HeaderMap,
    /// Optional JSON request body.
    pub body: Option<Value>,
}

impl FusenRequest {
    /// Removes and orders decoded arguments according to a method descriptor.
    pub fn take_arguments(
        &mut self,
        method: &MethodDescriptor,
    ) -> Result<Vec<ArgumentValue>, FusenError> {
        let body_count = method
            .parameters()
            .iter()
            .filter(|parameter| parameter.source() == ParameterSource::Body)
            .count();
        let mut body_values = match (body_count, self.body.take()) {
            (0, None) => VecDeque::new(),
            (0, Some(_)) => {
                return Err(FusenError::InvalidRequest(
                    "request body is not allowed for this method".into(),
                ));
            }
            (1, Some(value)) => VecDeque::from([value]),
            (1, None) => {
                return Err(FusenError::InvalidRequest(
                    "request body argument is missing".into(),
                ));
            }
            (_, Some(Value::Array(values))) if values.len() == body_count => values.into(),
            (_, Some(Value::Array(values))) => {
                return Err(FusenError::InvalidRequest(format!(
                    "request body argument count mismatch: expected {body_count}, got {}",
                    values.len()
                )));
            }
            (_, None) => {
                return Err(FusenError::InvalidRequest(
                    "request body arguments are missing".into(),
                ));
            }
            (_, Some(_)) => {
                return Err(FusenError::InvalidRequest(
                    "multiple body arguments require a JSON array".into(),
                ));
            }
        };

        let mut arguments = Vec::with_capacity(method.parameters().len());
        for parameter in method.parameters() {
            let argument = match parameter.source() {
                ParameterSource::Path => self
                    .path_parameters
                    .remove(parameter.name())
                    .map(ArgumentValue::Text)
                    .ok_or_else(|| {
                        FusenError::InvalidRequest(format!(
                            "path parameter {} is missing",
                            parameter.name()
                        ))
                    })?,
                ParameterSource::Query => {
                    let Some(values) = self.query_parameters.remove(parameter.name()) else {
                        arguments.push(ArgumentValue::Json(Value::Null));
                        continue;
                    };
                    if values.len() != 1 {
                        return Err(FusenError::InvalidRequest(format!(
                            "query parameter {} must appear exactly once",
                            parameter.name()
                        )));
                    }
                    ArgumentValue::Text(values.into_iter().next().ok_or_else(|| {
                        FusenError::InvalidRequest(format!(
                            "query parameter {} is missing",
                            parameter.name()
                        ))
                    })?)
                }
                ParameterSource::Body => body_values
                    .pop_front()
                    .map(ArgumentValue::Json)
                    .ok_or_else(|| {
                        FusenError::InvalidRequest("request body argument count mismatch".into())
                    })?,
                _ => {
                    return Err(FusenError::InvalidRequest(format!(
                        "parameter {} uses an unsupported source",
                        parameter.name()
                    )));
                }
            };
            arguments.push(argument);
        }
        if let Some(name) = self.path_parameters.keys().next() {
            return Err(FusenError::InvalidRequest(format!(
                "unexpected path parameter {name}"
            )));
        }
        if let Some(name) = self.query_parameters.keys().next() {
            return Err(FusenError::InvalidRequest(format!(
                "unexpected query parameter {name}"
            )));
        }
        if !body_values.is_empty() {
            return Err(FusenError::InvalidRequest(
                "request body argument count mismatch".into(),
            ));
        }
        Ok(arguments)
    }

    /// Builds a request from ordered client arguments and a method descriptor.
    pub fn init_request(
        protocol: WireProtocol,
        method: &MethodDescriptor,
        arguments: Vec<Value>,
    ) -> Result<Self, FusenError> {
        if arguments.len() != method.parameters().len() {
            return Err(FusenError::InvalidRequest(format!(
                "request argument count mismatch: expected {}, got {}",
                method.parameters().len(),
                arguments.len()
            )));
        }

        let mut path_parameters = HashMap::new();
        let mut query_parameters = QueryParameters::new();
        let mut body_values = Vec::new();
        for (parameter, value) in method.parameters().iter().zip(arguments) {
            match parameter.source() {
                ParameterSource::Path => {
                    if value.is_null() {
                        return Err(FusenError::InvalidRequest(format!(
                            "path parameter {} must not be null",
                            parameter.name()
                        )));
                    }
                    path_parameters.insert(parameter.name().to_owned(), value_to_text(value)?);
                }
                ParameterSource::Query => {
                    if !value.is_null() {
                        query_parameters
                            .entry(parameter.name().to_owned())
                            .or_default()
                            .push(value_to_text(value)?);
                    }
                }
                ParameterSource::Body => body_values.push(value),
                _ => {
                    return Err(FusenError::InvalidRequest(format!(
                        "parameter {} uses an unsupported source",
                        parameter.name()
                    )));
                }
            }
        }
        if protocol == WireProtocol::SpringCloud && body_values.len() > 1 {
            return Err(FusenError::InvalidRequest(
                "SpringCloud methods support at most one body parameter".into(),
            ));
        }
        let body = match body_values.len() {
            0 => None,
            1 => body_values.pop(),
            _ => Some(Value::Array(body_values)),
        };

        Ok(Self {
            protocol,
            path: Path {
                method: method.method().clone(),
                path: method.path().to_owned(),
            },
            endpoint: None,
            path_parameters,
            query_parameters,
            headers: HeaderMap::new(),
            body,
        })
    }
}

fn value_to_text(value: Value) -> Result<String, FusenError> {
    match value {
        Value::String(value) => Ok(value),
        value => serde_json::to_string(&value)
            .map_err(|error| FusenError::internal("failed to encode request argument", error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fusen_contract::{MethodId, ParameterDescriptor};

    fn parameter(name: &str, source: ParameterSource) -> ParameterDescriptor {
        ParameterDescriptor::__new(name, source).unwrap()
    }

    fn method(parameters: Vec<ParameterDescriptor>) -> MethodDescriptor {
        let query = parameters
            .iter()
            .any(|parameter| parameter.source() == ParameterSource::Query);
        let has_id_path = parameters.iter().any(|parameter| {
            parameter.name() == "id" && parameter.source() == ParameterSource::Path
        });
        MethodDescriptor::__new(
            MethodId::__new(0),
            "call",
            if query { Method::GET } else { Method::POST },
            if has_id_path { "/demo/{id}" } else { "/demo" },
            parameters,
        )
        .unwrap()
    }

    #[test]
    fn one_array_argument_keeps_its_boundary() {
        let method = method(vec![parameter("items", ParameterSource::Body)]);
        let mut request = FusenRequest::init_request(
            WireProtocol::Fusen,
            &method,
            vec![serde_json::json!([1, 2])],
        )
        .unwrap();
        let arguments = request.take_arguments(&method).unwrap();
        let value: Vec<i32> = arguments.into_iter().next().unwrap().deserialize().unwrap();
        assert_eq!(value, vec![1, 2]);
    }

    #[test]
    fn one_tuple_argument_keeps_its_boundary() {
        let method = method(vec![parameter("pair", ParameterSource::Body)]);
        let mut request = FusenRequest::init_request(
            WireProtocol::Fusen,
            &method,
            vec![serde_json::json!([1, "two"])],
        )
        .unwrap();
        let argument = request.take_arguments(&method).unwrap().remove(0);
        let value: (i32, String) = argument.deserialize().unwrap();
        assert_eq!(value, (1, "two".into()));
    }

    #[test]
    fn one_fixed_array_argument_keeps_its_boundary() {
        let method = method(vec![parameter("items", ParameterSource::Body)]);
        let mut request = FusenRequest::init_request(
            WireProtocol::Fusen,
            &method,
            vec![serde_json::json!([1, 2])],
        )
        .unwrap();
        let argument = request.take_arguments(&method).unwrap().remove(0);
        let value: [i32; 2] = argument.deserialize().unwrap();
        assert_eq!(value, [1, 2]);
    }

    #[test]
    fn text_deserialization_preserves_json_looking_strings() {
        let value: String = ArgumentValue::Text("\"quoted\"".into())
            .deserialize()
            .unwrap();
        assert_eq!(value, "\"quoted\"");
    }

    #[test]
    fn multiple_body_arguments_require_exact_array_length() {
        let method = method(vec![
            parameter("left", ParameterSource::Body),
            parameter("right", ParameterSource::Body),
        ]);
        let mut request = FusenRequest::init_request(
            WireProtocol::Fusen,
            &method,
            vec![serde_json::json!(1), serde_json::json!(2)],
        )
        .unwrap();
        assert_eq!(request.take_arguments(&method).unwrap().len(), 2);

        request.body = Some(serde_json::json!([1, 2, 3]));
        assert!(matches!(
            request.take_arguments(&method),
            Err(FusenError::InvalidRequest(_))
        ));
    }

    #[test]
    fn zero_body_arguments_reject_unexpected_body() {
        let method = method(Vec::new());
        let mut request =
            FusenRequest::init_request(WireProtocol::Fusen, &method, Vec::new()).unwrap();
        request.body = Some(Value::Null);
        assert!(request.take_arguments(&method).is_err());
    }

    #[test]
    fn missing_and_extra_parameters_are_rejected() {
        let method = method(vec![
            parameter("id", ParameterSource::Path),
            parameter("filter", ParameterSource::Query),
        ]);
        let arguments = vec![serde_json::json!("one"), serde_json::json!(null)];

        let mut missing =
            FusenRequest::init_request(WireProtocol::Fusen, &method, arguments.clone()).unwrap();
        missing.path_parameters.clear();
        assert!(matches!(
            missing.take_arguments(&method),
            Err(FusenError::InvalidRequest(_))
        ));

        let mut extra =
            FusenRequest::init_request(WireProtocol::Fusen, &method, arguments).unwrap();
        extra
            .query_parameters
            .insert("unexpected".into(), vec!["value".into()]);
        assert!(matches!(
            extra.take_arguments(&method),
            Err(FusenError::InvalidRequest(_))
        ));
    }

    #[test]
    fn absent_optional_query_round_trips_as_none() {
        let method = method(vec![parameter("filter", ParameterSource::Query)]);
        let mut request =
            FusenRequest::init_request(WireProtocol::Fusen, &method, vec![serde_json::json!(null)])
                .unwrap();
        let argument = request.take_arguments(&method).unwrap().remove(0);
        let value: Option<String> = argument.deserialize().unwrap();
        assert_eq!(value, None);
    }

    #[test]
    fn absent_required_query_is_rejected_during_deserialization() {
        let method = method(vec![parameter("filter", ParameterSource::Query)]);
        let mut request =
            FusenRequest::init_request(WireProtocol::Fusen, &method, vec![serde_json::json!(null)])
                .unwrap();
        let argument = request.take_arguments(&method).unwrap().remove(0);
        assert!(argument.deserialize::<String>().is_err());
    }

    #[test]
    fn null_path_parameter_is_rejected() {
        let method = method(vec![parameter("id", ParameterSource::Path)]);
        assert!(matches!(
            FusenRequest::init_request(WireProtocol::Fusen, &method, vec![serde_json::json!(null)]),
            Err(FusenError::InvalidRequest(_))
        ));
    }

    #[test]
    fn missing_single_body_is_rejected() {
        let method = method(vec![parameter("value", ParameterSource::Body)]);
        let mut request =
            FusenRequest::init_request(WireProtocol::Fusen, &method, vec![serde_json::json!(null)])
                .unwrap();
        request.body = None;
        assert!(matches!(
            request.take_arguments(&method),
            Err(FusenError::InvalidRequest(_))
        ));
    }
}
