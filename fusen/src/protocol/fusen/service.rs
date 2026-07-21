use crate::error::FusenError;
pub use fusen_contract::ParameterSource;
use http::Method;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct ServiceDesc {
    pub service_id: String,
    pub version: Option<String>,
    pub group: Option<String>,
    tag: String,
}

impl ServiceDesc {
    pub fn new(service_id: &str, version: Option<&str>, group: Option<&str>) -> Self {
        let tag = format!("{service_id}:{version:?}:{group:?}");
        Self {
            service_id: service_id.to_owned(),
            version: version.map(|e| e.to_owned()),
            group: group.map(|e| e.to_owned()),
            tag,
        }
    }

    pub fn get_tag(&self) -> &str {
        &self.tag
    }
}

#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub service_desc: ServiceDesc,
    pub method_name: String,
    pub method: Method,
    pub path: String,
    pub parameters: Vec<ParameterInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterInfo {
    pub name: String,
    pub source: ParameterSource,
}

impl ParameterInfo {
    pub fn new(name: impl Into<String>, source: ParameterSource) -> Self {
        Self {
            name: name.into(),
            source,
        }
    }
}

impl MethodInfo {
    pub fn new(
        service_desc: ServiceDesc,
        method_name: String,
        method: Method,
        path: String,
        parameters: Vec<ParameterInfo>,
    ) -> Self {
        Self {
            service_desc,
            method_name,
            method,
            path,
            parameters,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub service_desc: ServiceDesc,
    pub method_infos: Vec<MethodInfo>,
}

impl ServiceInfo {
    pub fn new(service_desc: ServiceDesc, method_infos: Vec<MethodInfo>) -> Self {
        Self {
            service_desc,
            method_infos,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), FusenError> {
        if self.service_desc.service_id.trim().is_empty() {
            return Err(FusenError::InvalidRequest(
                "service id must not be empty".into(),
            ));
        }
        if self.method_infos.is_empty() {
            return Err(FusenError::InvalidRequest(format!(
                "service {} must declare at least one method",
                self.service_desc.service_id
            )));
        }
        let mut method_names = HashSet::new();
        for method in &self.method_infos {
            if method.service_desc.get_tag() != self.service_desc.get_tag() {
                return Err(FusenError::InvalidRequest(format!(
                    "method {} belongs to a different service",
                    method.method_name
                )));
            }
            if method.method_name.is_empty() || !method_names.insert(method.method_name.as_str()) {
                return Err(FusenError::InvalidRequest(format!(
                    "service {} contains an empty or duplicate method name",
                    self.service_desc.service_id
                )));
            }
            validate_method(method)?;
        }
        Ok(())
    }
}

fn validate_method(method: &MethodInfo) -> Result<(), FusenError> {
    if !matches!(
        method.method,
        Method::GET
            | Method::POST
            | Method::PUT
            | Method::PATCH
            | Method::DELETE
            | Method::HEAD
            | Method::OPTIONS
    ) {
        return Err(FusenError::InvalidRequest(format!(
            "method {} uses an unsupported HTTP method",
            method.method_name
        )));
    }
    if !method.path.starts_with('/') || method.path.contains(['?', '#']) {
        return Err(FusenError::InvalidRequest(format!(
            "method {} has an invalid route {}",
            method.method_name, method.path
        )));
    }

    let mut placeholders = HashSet::new();
    for segment in method
        .path
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
    {
        if let Some(name) = segment
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        {
            if name.is_empty() || !placeholders.insert(name) || name.contains(['{', '}']) {
                return Err(FusenError::InvalidRequest(format!(
                    "method {} has an invalid or duplicate route parameter",
                    method.method_name
                )));
            }
        } else if segment.contains(['{', '}']) {
            return Err(FusenError::InvalidRequest(format!(
                "method {} has a malformed route parameter",
                method.method_name
            )));
        }
    }

    let query_method = matches!(method.method, Method::GET | Method::DELETE | Method::HEAD);
    let mut parameter_names = HashSet::new();
    let mut path_parameters = HashSet::new();
    for parameter in &method.parameters {
        if parameter.name.is_empty() || !parameter_names.insert(parameter.name.as_str()) {
            return Err(FusenError::InvalidRequest(format!(
                "method {} contains an empty or duplicate parameter name",
                method.method_name
            )));
        }
        match parameter.source {
            ParameterSource::Path => {
                path_parameters.insert(parameter.name.as_str());
            }
            ParameterSource::Query if !query_method => {
                return Err(FusenError::InvalidRequest(format!(
                    "non-path parameter {} must use Body for {}",
                    parameter.name, method.method
                )));
            }
            ParameterSource::Body if query_method => {
                return Err(FusenError::InvalidRequest(format!(
                    "non-path parameter {} must use Query for {}",
                    parameter.name, method.method
                )));
            }
            ParameterSource::Query | ParameterSource::Body => {}
            _ => {
                return Err(FusenError::InvalidRequest(format!(
                    "method {} uses an unsupported parameter source",
                    method.method_name
                )));
            }
        }
    }
    if placeholders != path_parameters {
        return Err(FusenError::InvalidRequest(format!(
            "method {} route parameters do not match its Path metadata",
            method.method_name
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_validation_rejects_duplicate_placeholders() {
        let service = ServiceDesc::new("demo", None, None);
        let info = ServiceInfo::new(
            service.clone(),
            vec![MethodInfo::new(
                service,
                "find".into(),
                Method::GET,
                "/users/{id}/{id}".into(),
                vec![ParameterInfo::new("id", ParameterSource::Path)],
            )],
        );
        assert!(matches!(
            info.validate(),
            Err(FusenError::InvalidRequest(_))
        ));
    }

    #[test]
    fn service_validation_rejects_wrong_parameter_source() {
        let service = ServiceDesc::new("demo", None, None);
        let info = ServiceInfo::new(
            service.clone(),
            vec![MethodInfo::new(
                service,
                "find".into(),
                Method::GET,
                "/users".into(),
                vec![ParameterInfo::new("filter", ParameterSource::Body)],
            )],
        );
        assert!(matches!(
            info.validate(),
            Err(FusenError::InvalidRequest(_))
        ));
    }
}
