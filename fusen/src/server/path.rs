use crate::{
    error::FusenError,
    protocol::fusen::{
        request::Path,
        service::{MethodInfo, ParameterSource},
    },
};
use http::Method;
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Default)]
pub struct PathCache {
    methods: HashMap<Method, RouteNode>,
}

#[derive(Debug, Default)]
struct RouteNode {
    literals: HashMap<String, RouteNode>,
    parameter: Option<Box<RouteNode>>,
    target: Option<RouteTarget>,
}

#[derive(Debug)]
struct RouteTarget {
    method_info: Arc<MethodInfo>,
    path_parameters: Vec<(usize, String)>,
}

#[derive(Debug)]
pub struct QueryResult {
    pub method_info: Arc<MethodInfo>,
    pub path_parameters: HashMap<String, String>,
}

impl PathCache {
    pub fn build(method_infos: Vec<Arc<MethodInfo>>) -> Result<Self, FusenError> {
        let mut cache = Self::default();
        for method_info in method_infos {
            cache.insert(method_info)?;
        }
        Ok(cache)
    }

    fn insert(&mut self, method_info: Arc<MethodInfo>) -> Result<(), FusenError> {
        let (segments, path_parameters) = parse_template(&method_info)?;
        let mut node = self.methods.entry(method_info.method.clone()).or_default();
        for segment in segments {
            node = match segment {
                TemplateSegment::Literal(value) => node.literals.entry(value).or_default(),
                TemplateSegment::Parameter => node
                    .parameter
                    .get_or_insert_with(|| Box::new(RouteNode::default())),
            };
        }
        if let Some(existing) = &node.target {
            return Err(FusenError::InvalidRequest(format!(
                "ambiguous routes {} and {}",
                existing.method_info.path, method_info.path
            )));
        }
        node.target = Some(RouteTarget {
            method_info,
            path_parameters,
        });
        Ok(())
    }

    pub fn search(&self, path: &Path) -> Result<Option<QueryResult>, FusenError> {
        let Some(root) = self.methods.get(&path.method) else {
            return Ok(None);
        };
        let segments = decode_path_segments(&path.path)?;
        let Some(target) = find_target(root, &segments, 0) else {
            return Ok(None);
        };
        let mut path_parameters = HashMap::new();
        for (index, name) in &target.path_parameters {
            path_parameters.insert(name.clone(), segments[*index].clone());
        }
        Ok(Some(QueryResult {
            method_info: target.method_info.clone(),
            path_parameters,
        }))
    }
}

#[derive(Debug)]
enum TemplateSegment {
    Literal(String),
    Parameter,
}

type ParsedTemplate = (Vec<TemplateSegment>, Vec<(usize, String)>);

fn parse_template(method: &MethodInfo) -> Result<ParsedTemplate, FusenError> {
    if !method.path.starts_with('/') {
        return Err(FusenError::InvalidRequest(format!(
            "route must start with '/': {}",
            method.path
        )));
    }
    let raw_segments = split_path(&method.path);
    let mut segments = Vec::with_capacity(raw_segments.len());
    let mut names = Vec::new();
    for (index, raw) in raw_segments.into_iter().enumerate() {
        if let Some(name) = raw
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        {
            if name.is_empty() || name.contains(['{', '}']) {
                return Err(FusenError::InvalidRequest(format!(
                    "invalid route parameter in {}",
                    method.path
                )));
            }
            if names.iter().any(|(_, existing)| existing == name) {
                return Err(FusenError::InvalidRequest(format!(
                    "duplicate route parameter {name} in {}",
                    method.path
                )));
            }
            names.push((index, name.to_owned()));
            segments.push(TemplateSegment::Parameter);
        } else if raw.contains(['{', '}']) {
            return Err(FusenError::InvalidRequest(format!(
                "invalid route parameter in {}",
                method.path
            )));
        } else {
            segments.push(TemplateSegment::Literal(raw.to_owned()));
        }
    }

    for (_, name) in &names {
        if !method
            .parameters
            .iter()
            .any(|parameter| parameter.name == *name && parameter.source == ParameterSource::Path)
        {
            return Err(FusenError::InvalidRequest(format!(
                "route parameter {name} has no matching Path parameter"
            )));
        }
    }
    for parameter in method
        .parameters
        .iter()
        .filter(|parameter| parameter.source == ParameterSource::Path)
    {
        if !names.iter().any(|(_, name)| *name == parameter.name) {
            return Err(FusenError::InvalidRequest(format!(
                "Path parameter {} is not present in route {}",
                parameter.name, method.path
            )));
        }
    }
    Ok((segments, names))
}

fn split_path(path: &str) -> Vec<&str> {
    path.trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn decode_path_segments(path: &str) -> Result<Vec<String>, FusenError> {
    split_path(path)
        .into_iter()
        .map(|segment| {
            validate_percent_encoding(segment)?;
            urlencoding::decode(segment)
                .map(|value| value.into_owned())
                .map_err(|error| FusenError::InvalidRequest(error.to_string()))
        })
        .collect()
}

fn validate_percent_encoding(segment: &str) -> Result<(), FusenError> {
    let bytes = segment.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return Err(FusenError::InvalidRequest(format!(
                    "invalid percent encoding in path segment {segment:?}"
                )));
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    Ok(())
}

fn find_target<'a>(
    node: &'a RouteNode,
    segments: &[String],
    index: usize,
) -> Option<&'a RouteTarget> {
    if index == segments.len() {
        return node.target.as_ref();
    }
    if let Some(literal) = node.literals.get(&segments[index])
        && let Some(target) = find_target(literal, segments, index + 1)
    {
        return Some(target);
    }
    node.parameter
        .as_deref()
        .and_then(|parameter| find_target(parameter, segments, index + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::fusen::service::{ParameterInfo, ServiceDesc};

    fn method(name: &str, path: &str) -> Arc<MethodInfo> {
        let parameters = path
            .trim_matches('/')
            .split('/')
            .filter_map(|segment| {
                segment
                    .strip_prefix('{')
                    .and_then(|value| value.strip_suffix('}'))
                    .map(|name| ParameterInfo::new(name, ParameterSource::Path))
            })
            .collect();
        Arc::new(MethodInfo::new(
            ServiceDesc::new("demo", None, None),
            name.into(),
            Method::GET,
            path.into(),
            parameters,
        ))
    }

    #[test]
    fn rejects_equivalent_dynamic_routes() {
        let error = PathCache::build(vec![
            method("by_id", "/users/{id}"),
            method("by_name", "/users/{name}"),
        ])
        .unwrap_err();
        assert!(matches!(error, FusenError::InvalidRequest(_)));
    }

    #[test]
    fn static_segments_win_independently_of_insertion_order() {
        for routes in [
            vec![method("dynamic", "/a/{x}/c"), method("static", "/a/b/{y}")],
            vec![method("static", "/a/b/{y}"), method("dynamic", "/a/{x}/c")],
        ] {
            let cache = PathCache::build(routes).unwrap();
            let result = cache
                .search(&Path {
                    method: Method::GET,
                    path: "/a/b/c".into(),
                })
                .unwrap()
                .unwrap();
            assert_eq!(result.method_info.method_name, "static");
        }
    }

    #[test]
    fn decodes_dynamic_parameter() {
        let cache = PathCache::build(vec![method("user", "/users/{id}")]).unwrap();
        let result = cache
            .search(&Path {
                method: Method::GET,
                path: "/users/a%2Fb%20c".into(),
            })
            .unwrap()
            .unwrap();
        assert_eq!(result.path_parameters["id"], "a/b c");
    }

    #[test]
    fn decodes_static_unicode_spaces_and_literal_percent() {
        let cache = PathCache::build(vec![method("literal", "/café/hello world/%")]).unwrap();
        let result = cache
            .search(&Path {
                method: Method::GET,
                path: "/caf%C3%A9/hello%20world/%25".into(),
            })
            .unwrap()
            .unwrap();
        assert_eq!(result.method_info.method_name, "literal");
    }

    #[test]
    fn rejects_malformed_percent_encoding() {
        let cache = PathCache::build(vec![method("user", "/users/{id}")]).unwrap();
        for path in ["/users/%", "/users/%2", "/users/%GG", "/users/%FF"] {
            assert!(matches!(
                cache.search(&Path {
                    method: Method::GET,
                    path: path.into(),
                }),
                Err(FusenError::InvalidRequest(_))
            ));
        }
    }

    #[test]
    fn decoded_static_segment_still_wins() {
        let cache = PathCache::build(vec![
            method("dynamic", "/files/{name}"),
            method("static", "/files/static"),
        ])
        .unwrap();
        let result = cache
            .search(&Path {
                method: Method::GET,
                path: "/files/%73tatic".into(),
            })
            .unwrap()
            .unwrap();
        assert_eq!(result.method_info.method_name, "static");
    }
}
