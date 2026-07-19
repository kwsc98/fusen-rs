use crate::{
    error::FusenError,
    protocol::fusen::{request::Path, service::MethodInfo},
};
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Default)]
pub struct PathCache {
    exact: HashMap<String, Arc<MethodInfo>>,
    dynamic: Vec<DynamicRoute>,
}

#[derive(Debug)]
struct DynamicRoute {
    method: http::Method,
    segments: Vec<RouteSegment>,
    canonical: String,
    value: Arc<MethodInfo>,
}

#[derive(Debug)]
enum RouteSegment {
    Literal(String),
    Parameter(String),
}

#[derive(Debug)]
pub struct QueryResult {
    pub method_info: Arc<MethodInfo>,
    pub rest_fields: Option<Vec<(String, String)>>,
}

impl PathCache {
    pub fn build(method_infos: Vec<Arc<MethodInfo>>) -> Result<Self, FusenError> {
        let mut cache = Self::default();
        let mut dynamic_keys = HashMap::<String, String>::new();
        for method_info in method_infos {
            let key = format!("{}:{}", method_info.method, method_info.path);
            if cache.exact.contains_key(&key) {
                return Err(FusenError::InvalidRequest(format!("duplicate route {key}")));
            }
            if method_info
                .path
                .split('/')
                .any(|segment| segment.starts_with('{') && segment.ends_with('}'))
            {
                let route = DynamicRoute::new(method_info.clone())?;
                let canonical_key = format!("{}:{}", route.method, route.canonical);
                if let Some(previous) = dynamic_keys.insert(canonical_key.clone(), key.clone()) {
                    return Err(FusenError::InvalidRequest(format!(
                        "ambiguous routes {previous} and {key} ({canonical_key})"
                    )));
                }
                cache.dynamic.push(route);
            }
            cache.exact.insert(key, method_info);
        }
        cache
            .dynamic
            .sort_by_key(|route| std::cmp::Reverse(route.literal_count()));
        Ok(cache)
    }

    pub fn search(&self, path: &Path) -> Option<QueryResult> {
        let key = format!("{}:{}", path.method, path.path);
        if let Some(method_info) = self.exact.get(&key) {
            return Some(QueryResult {
                method_info: method_info.clone(),
                rest_fields: None,
            });
        }
        self.dynamic.iter().find_map(|route| route.matches(path))
    }
}

impl DynamicRoute {
    fn new(value: Arc<MethodInfo>) -> Result<Self, FusenError> {
        let mut segments = Vec::new();
        let mut canonical = String::new();
        for raw in value.path.trim_matches('/').split('/') {
            canonical.push('/');
            if raw.starts_with('{') && raw.ends_with('}') {
                let name = &raw[1..raw.len() - 1];
                if name.is_empty() {
                    return Err(FusenError::InvalidRequest(format!(
                        "empty route parameter in {}",
                        value.path
                    )));
                }
                canonical.push_str("{}");
                segments.push(RouteSegment::Parameter(name.to_owned()));
            } else if raw.contains('{') || raw.contains('}') {
                return Err(FusenError::InvalidRequest(format!(
                    "invalid route parameter in {}",
                    value.path
                )));
            } else {
                canonical.push_str(raw);
                segments.push(RouteSegment::Literal(raw.to_owned()));
            }
        }
        Ok(Self {
            method: value.method.clone(),
            segments,
            canonical,
            value,
        })
    }

    fn literal_count(&self) -> usize {
        self.segments
            .iter()
            .filter(|segment| matches!(segment, RouteSegment::Literal(_)))
            .count()
    }

    fn matches(&self, path: &Path) -> Option<QueryResult> {
        if self.method != path.method {
            return None;
        }
        let values = path.path.trim_matches('/').split('/').collect::<Vec<_>>();
        if values.len() != self.segments.len() {
            return None;
        }
        let mut fields = Vec::new();
        for (segment, value) in self.segments.iter().zip(values) {
            match segment {
                RouteSegment::Literal(expected) if expected != value => return None,
                RouteSegment::Literal(_) => {}
                RouteSegment::Parameter(name) => fields.push((name.clone(), value.to_owned())),
            }
        }
        Some(QueryResult {
            method_info: self.value.clone(),
            rest_fields: (!fields.is_empty()).then_some(fields),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::fusen::service::ServiceDesc;

    fn method(path: &str) -> Arc<MethodInfo> {
        Arc::new(MethodInfo::new(
            ServiceDesc::new("demo", None, None),
            path.into(),
            "GET".into(),
            path.into(),
            Vec::new(),
        ))
    }

    #[test]
    fn rejects_equivalent_dynamic_routes() {
        let error =
            PathCache::build(vec![method("/users/{id}"), method("/users/{name}")]).unwrap_err();
        assert!(matches!(error, FusenError::InvalidRequest(_)));
    }

    #[test]
    fn extracts_dynamic_parameter() {
        let cache = PathCache::build(vec![method("/users/{id}")]).unwrap();
        let result = cache
            .search(&Path {
                method: http::Method::GET,
                path: "/users/42".into(),
            })
            .unwrap();
        assert_eq!(
            result.rest_fields.unwrap(),
            vec![("id".into(), "42".into())]
        );
    }
}
