use crate::args::{MethodArgs, ServiceArgs};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use syn::visit::Visit;
use syn::{
    Attribute, FnArg, GenericArgument, GenericParam, Generics, Ident, ItemTrait, Meta, Pat,
    PathArguments, ReturnType, Signature, TraitItem, Type, TypePath, WherePredicate,
};

const MAX_IDENTITY_BYTES: usize = 128;

pub(crate) struct HttpMapping {
    pub(crate) method: String,
    pub(crate) path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParameterSource {
    Context,
    Path,
    Query,
    BodyField,
    Body,
}

#[derive(Clone)]
pub(crate) struct Parameter {
    pub(crate) ident: Ident,
    pub(crate) kind: Type,
    pub(crate) wire_name: String,
    pub(crate) source: ParameterSource,
    pub(crate) repeated: bool,
    pub(crate) parse_spring_json_primitive: bool,
}

pub(crate) struct Method {
    pub(crate) ident: Ident,
    pub(crate) parameters: Vec<Parameter>,
    pub(crate) response: Type,
    pub(crate) http: HttpMapping,
}

pub(crate) struct Service {
    pub(crate) name: String,
    pub(crate) group: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) methods: Vec<Method>,
}

pub(crate) fn validate(args: ServiceArgs, item: &ItemTrait) -> syn::Result<Service> {
    if item.unsafety.is_some() || item.modifiers.auto_token.is_some() {
        return Err(syn::Error::new_spanned(
            &item.ident,
            "RPC interfaces must be ordinary safe traits",
        ));
    }
    validate_trait_generics(&item.generics)?;
    reject_conditional_attributes(&item.attrs, "RPC interface traits")?;

    let name = args.name.ok_or_else(|| {
        syn::Error::new(
            item.ident.span(),
            "interface name is required: use `#[interface(name = \"...\")]`",
        )
    })?;
    validate_identity(&name.value(), "interface name", name.span())?;
    if let Some(group) = &args.group {
        validate_identity(&group.value(), "interface group", group.span())?;
    }
    if let Some(version) = &args.version {
        validate_identity(&version.value(), "interface version", version.span())?;
    }
    if item.items.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.ident,
            "RPC interfaces must declare at least one method",
        ));
    }
    if item.items.len() > usize::from(u16::MAX) + 1 {
        return Err(syn::Error::new_spanned(
            &item.ident,
            "RPC interfaces may declare at most 65536 methods",
        ));
    }

    let mut methods = Vec::with_capacity(item.items.len());
    let mut http_routes = BTreeMap::<(String, String), Ident>::new();
    for trait_item in &item.items {
        let TraitItem::Fn(method) = trait_item else {
            return Err(syn::Error::new_spanned(
                trait_item,
                "RPC interfaces may contain only async methods",
            ));
        };
        reject_conditional_attributes(&method.attrs, "RPC methods")?;
        if method.default.is_some() {
            return Err(syn::Error::new_spanned(
                method,
                "RPC methods must not provide default implementations",
            ));
        }
        validate_signature(&method.sig)?;
        validate_identity(
            method.sig.ident.to_string().trim_start_matches("r#"),
            "method name",
            method.sig.ident.span(),
        )?;
        let method_args = method_args(&method.attrs, &method.sig.ident)?;
        let http = validate_http(method_args, &method.sig.ident)?;
        let parameters = parameters(&method.sig, &http)?;
        let response = response_type(&method.sig.output)?;
        if http.method == "HEAD"
            && !matches!(&response, Type::Tuple(tuple) if tuple.elems.is_empty())
        {
            return Err(syn::Error::new_spanned(
                &method.sig.output,
                "HTTP HEAD mappings must return Result<RpcResponse<()>, RpcError>",
            ));
        }
        let key = (http.method.clone(), route_shape(&http.path));
        if let Some(first) = http_routes.insert(key, method.sig.ident.clone()) {
            return Err(syn::Error::new(
                method.sig.ident.span(),
                format!(
                    "duplicate HTTP route {} {}; it conflicts with method `{first}`",
                    http.method, http.path
                ),
            ));
        }
        methods.push(Method {
            ident: method.sig.ident.clone(),
            parameters,
            response,
            http,
        });
    }

    Ok(Service {
        name: name.value(),
        group: args.group.map(|value| value.value()),
        version: args.version.map(|value| value.value()),
        methods,
    })
}

pub(crate) fn is_method_attr(attribute: &Attribute) -> bool {
    attribute
        .path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "method")
}

fn method_args(attributes: &[Attribute], method_ident: &Ident) -> syn::Result<MethodArgs> {
    let mut parsed = None;
    for attribute in attributes
        .iter()
        .filter(|attribute| is_method_attr(attribute))
    {
        if parsed.is_some() {
            return Err(syn::Error::new_spanned(
                attribute,
                "only one `method` attribute is allowed per RPC method",
            ));
        }
        let args = match &attribute.meta {
            Meta::Path(_) => MethodArgs::default(),
            Meta::List(list) => MethodArgs::parse_tokens(list.tokens.clone())?,
            Meta::NameValue(_) => {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "`method` must use `#[method(...)]` syntax",
                ));
            }
        };
        parsed = Some(args);
    }
    parsed.ok_or_else(|| {
        syn::Error::new(
            method_ident.span(),
            "each RPC method must declare #[method(method = \"...\", path = \"/...\")]",
        )
    })
}

fn validate_trait_generics(generics: &Generics) -> syn::Result<()> {
    if !generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &generics.params,
            "RPC interfaces must not declare generic parameters",
        ));
    }
    if let Some(where_clause) = &generics.where_clause {
        for predicate in &where_clause.predicates {
            let WherePredicate::Type(predicate) = predicate else {
                return Err(syn::Error::new_spanned(
                    predicate,
                    "RPC interface where clauses may constrain only Self",
                ));
            };
            let Type::Path(TypePath {
                qself: None, path, ..
            }) = &predicate.bounded_ty
            else {
                return Err(syn::Error::new_spanned(
                    predicate,
                    "RPC interface where clauses may constrain only Self",
                ));
            };
            if !path.is_ident("Self") {
                return Err(syn::Error::new_spanned(
                    predicate,
                    "RPC interface where clauses may constrain only Self",
                ));
            }
        }
    }
    Ok(())
}

fn validate_signature(signature: &Signature) -> syn::Result<()> {
    if signature.asyncness.is_none()
        || signature.constness.is_some()
        || matches!(signature.safety, syn::Safety::Unsafe(_))
        || signature.abi.is_some()
        || signature.variadic.is_some()
    {
        return Err(syn::Error::new_spanned(
            signature,
            "RPC methods must be ordinary async Rust methods",
        ));
    }
    if signature.generics.params.iter().any(|parameter| {
        matches!(
            parameter,
            GenericParam::Lifetime(_) | GenericParam::Type(_) | GenericParam::Const(_)
        )
    }) || signature.generics.where_clause.is_some()
    {
        return Err(syn::Error::new_spanned(
            &signature.generics,
            "RPC methods must not declare generic parameters or where clauses",
        ));
    }
    let Some(FnArg::Receiver(receiver)) = signature.inputs.first() else {
        return Err(syn::Error::new_spanned(
            signature,
            "RPC methods must have an immutable &self receiver",
        ));
    };
    if !matches!(&receiver.kind, syn::ReceiverKind::Reference(_, None, None))
        || receiver.mutability.is_some()
    {
        return Err(syn::Error::new_spanned(
            receiver,
            "RPC methods must have an immutable &self receiver without an explicit lifetime",
        ));
    }
    for input in signature.inputs.iter().skip(1) {
        let FnArg::Typed(input) = input else {
            return Err(syn::Error::new_spanned(
                input,
                "RPC methods may declare only one receiver",
            ));
        };
        validate_owned_type(&input.ty)?;
    }

    Ok(())
}

fn parameters(signature: &Signature, http: &HttpMapping) -> syn::Result<Vec<Parameter>> {
    let mut parameters = Vec::with_capacity(signature.inputs.len().saturating_sub(1));
    let mut names = BTreeSet::new();
    let placeholders = validate_route(&http.path, signature.ident.span())?;
    let body_fields_by_default = matches!(http.method.as_str(), "POST" | "PUT" | "PATCH");
    let mut raw_body = None;
    let mut first_body_field = None;
    let mut context = None;

    for input in signature.inputs.iter().skip(1) {
        let FnArg::Typed(input) = input else {
            unreachable!("the signature validator rejected extra receivers")
        };
        let Pat::Ident(pattern) = input.pat.as_ref() else {
            return Err(syn::Error::new_spanned(
                &input.pat,
                "RPC parameters must use plain immutable identifier patterns",
            ));
        };
        if pattern.by_ref.is_some() || pattern.mutability.is_some() || pattern.subpat.is_some() {
            return Err(syn::Error::new_spanned(
                pattern,
                "RPC parameters must use plain immutable identifier patterns",
            ));
        }

        if let Some(attribute) = input.attrs.iter().find(|attribute| is_rpc_attr(attribute)) {
            return Err(syn::Error::new_spanned(
                attribute,
                "`#[rpc(...)]` was removed; use `#[param(...)]`",
            ));
        }

        let mut explicit_source = None;
        let mut wire_name = None;
        for attribute in input
            .attrs
            .iter()
            .filter(|attribute| is_param_attr(attribute))
        {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("context")
                    || meta.path.is_ident("query")
                    || meta.path.is_ident("body")
                {
                    if explicit_source.is_some() {
                        return Err(meta
                            .error("a parameter may declare only one of context, query, or body"));
                    }
                    explicit_source = Some(if meta.path.is_ident("context") {
                        ParameterSource::Context
                    } else if meta.path.is_ident("query") {
                        ParameterSource::Query
                    } else {
                        ParameterSource::Body
                    });
                    Ok(())
                } else if meta.path.is_ident("name") {
                    if wire_name.is_some() {
                        return Err(meta.error("duplicate parameter name"));
                    }
                    wire_name = Some(meta.value()?.parse::<syn::LitStr>()?);
                    Ok(())
                } else {
                    Err(meta
                        .error("unknown parameter field; expected context, query, body, or name"))
                }
            })?;
        }
        let rust_name = pattern
            .ident
            .to_string()
            .trim_start_matches("r#")
            .to_owned();

        if explicit_source == Some(ParameterSource::Context) {
            if wire_name.is_some() {
                return Err(syn::Error::new_spanned(
                    input,
                    "#[param(context)] parameters must not declare a wire name",
                ));
            }
            if context.replace(pattern.ident.span()).is_some() {
                return Err(syn::Error::new_spanned(
                    input,
                    "an RPC method may declare at most one #[param(context)] parameter",
                ));
            }
            let Type::Path(kind) = input.ty.as_ref() else {
                return Err(syn::Error::new_spanned(
                    &input.ty,
                    "#[param(context)] parameters must use the RpcCall type",
                ));
            };
            if !is_runtime_type_path(&kind.path, "RpcCall") {
                return Err(syn::Error::new_spanned(
                    &input.ty,
                    "#[param(context)] parameters must use the RpcCall type",
                ));
            }
            parameters.push(Parameter {
                ident: pattern.ident.clone(),
                kind: input.ty.as_ref().clone(),
                wire_name: String::new(),
                source: ParameterSource::Context,
                repeated: false,
                parse_spring_json_primitive: false,
            });
            continue;
        }

        let wire_name = wire_name.map_or_else(|| rust_name.clone(), |name| name.value());
        validate_identity(&wire_name, "RPC parameter name", pattern.ident.span())?;
        if !names.insert(wire_name.clone()) {
            return Err(syn::Error::new_spanned(
                input,
                format!("duplicate RPC wire parameter name `{wire_name}`"),
            ));
        }

        let source = explicit_source.unwrap_or_else(|| {
            if placeholders.contains(&wire_name) {
                ParameterSource::Path
            } else if body_fields_by_default {
                ParameterSource::BodyField
            } else {
                ParameterSource::Query
            }
        });
        if source == ParameterSource::Body
            && matches!(http.method.as_str(), "GET" | "HEAD" | "OPTIONS")
        {
            return Err(syn::Error::new_spanned(
                input,
                format!("{} methods do not accept a JSON request body", http.method),
            ));
        }
        if source == ParameterSource::Body && raw_body.replace(pattern.ident.span()).is_some() {
            return Err(syn::Error::new_spanned(
                input,
                "an RPC method may declare at most one #[param(body)] parameter",
            ));
        }
        if source == ParameterSource::BodyField && first_body_field.is_none() {
            first_body_field = Some(pattern.ident.span());
        }
        let repeated = if source == ParameterSource::Query {
            if direct_generic_type(&input.ty, "Option")
                .and_then(|inner| direct_generic_type(inner, "Vec"))
                .is_some()
            {
                return Err(syn::Error::new_spanned(
                    &input.ty,
                    "query parameters may not use Option<Vec<T>>; use Vec<T> so omission has one unambiguous empty-list meaning",
                ));
            }
            direct_generic_type(&input.ty, "Vec").is_some()
        } else {
            if source == ParameterSource::Path
                && (direct_generic_type(&input.ty, "Option").is_some()
                    || direct_generic_type(&input.ty, "Vec").is_some())
            {
                return Err(syn::Error::new_spanned(
                    &input.ty,
                    "path parameters must be required scalar values",
                ));
            }
            false
        };
        let scalar_type = direct_generic_type(&input.ty, "Option").unwrap_or(&input.ty);
        let scalar_type = direct_generic_type(scalar_type, "Vec").unwrap_or(scalar_type);
        parameters.push(Parameter {
            ident: pattern.ident.clone(),
            kind: input.ty.as_ref().clone(),
            wire_name,
            source,
            repeated,
            parse_spring_json_primitive: matches!(
                source,
                ParameterSource::Path | ParameterSource::Query
            ) && is_json_primitive_type(scalar_type),
        });
    }

    for placeholder in &placeholders {
        if !parameters.iter().any(|parameter| {
            parameter.source == ParameterSource::Path && parameter.wire_name == *placeholder
        }) {
            return Err(syn::Error::new(
                signature.ident.span(),
                format!(
                    "HTTP path parameter `{placeholder}` requires a method parameter with the same wire name"
                ),
            ));
        }
    }
    if raw_body.is_some()
        && let Some(span) = first_body_field
    {
        return Err(syn::Error::new(
            span,
            "default body fields cannot be combined with a #[param(body)] raw body; mark the other parameters #[param(query)]",
        ));
    }

    Ok(parameters)
}

fn response_type(output: &ReturnType) -> syn::Result<Type> {
    let ReturnType::Type(_, output) = output else {
        return Err(syn::Error::new_spanned(
            output,
            "RPC methods must return Result<RpcResponse<T>, RpcError>",
        ));
    };
    let Type::Path(result) = output.as_ref() else {
        return Err(syn::Error::new_spanned(
            output,
            "RPC methods must return Result<RpcResponse<T>, RpcError>",
        ));
    };
    let segment = result.path.segments.last().ok_or_else(|| {
        syn::Error::new_spanned(
            output,
            "RPC methods must return Result<RpcResponse<T>, RpcError>",
        )
    })?;
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Err(syn::Error::new_spanned(
            output,
            "RPC methods must return Result<RpcResponse<T>, RpcError>",
        ));
    };
    if !is_standard_result_path(&result.path) || arguments.args.len() != 2 {
        return Err(syn::Error::new_spanned(
            output,
            "RPC methods must return Result<RpcResponse<T>, RpcError>",
        ));
    }
    let mut args = arguments.args.iter();
    let Some(GenericArgument::Type(success)) = args.next() else {
        unreachable!()
    };
    let response = wrapper_inner(success, "RpcResponse", "success type")?;
    validate_owned_type(&response)?;
    let Some(GenericArgument::Type(Type::Path(error))) = args.next() else {
        return Err(syn::Error::new_spanned(
            output,
            "RPC methods must use RpcError as their error type",
        ));
    };
    if !is_runtime_type_path(&error.path, "RpcError") {
        return Err(syn::Error::new_spanned(
            error,
            "RPC methods must use RpcError as their error type",
        ));
    }
    Ok(response)
}

fn is_standard_result_path(path: &syn::Path) -> bool {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    matches!(segments.as_slice(), [result] if result == "Result")
        || matches!(segments.as_slice(), [root, module, result]
            if matches!(root.as_str(), "std" | "core")
                && module == "result"
                && result == "Result")
}

fn is_runtime_type_path(path: &syn::Path, expected: &str) -> bool {
    if path
        .segments
        .iter()
        .any(|segment| !segment.arguments.is_empty())
    {
        return false;
    }
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    matches!(segments.as_slice(), [actual] if actual == expected)
        || matches!(segments.as_slice(), [runtime, actual]
            if runtime == &crate::runtime_crate_name() && actual == expected)
}

fn wrapper_inner(kind: &Type, wrapper: &str, position: &str) -> syn::Result<Type> {
    let Type::Path(path) = kind else {
        return Err(syn::Error::new_spanned(
            kind,
            format!("RPC {position} must be {wrapper}<T>"),
        ));
    };
    let Some(segment) = path.path.segments.last() else {
        return Err(syn::Error::new_spanned(
            kind,
            format!("RPC {position} must be {wrapper}<T>"),
        ));
    };
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Err(syn::Error::new_spanned(
            kind,
            format!("RPC {position} must be {wrapper}<T>"),
        ));
    };
    if segment.ident != wrapper || arguments.args.len() != 1 {
        return Err(syn::Error::new_spanned(
            kind,
            format!("RPC {position} must be {wrapper}<T>"),
        ));
    }
    match arguments.args.first() {
        Some(GenericArgument::Type(kind)) => Ok(kind.clone()),
        _ => Err(syn::Error::new_spanned(
            kind,
            format!("RPC {position} must be {wrapper}<T>"),
        )),
    }
}

fn validate_http(args: MethodArgs, method_ident: &Ident) -> syn::Result<HttpMapping> {
    let method = args.method.ok_or_else(|| {
        syn::Error::new(
            method_ident.span(),
            "method attribute requires method = \"...\"",
        )
    })?;
    let path = args.path.ok_or_else(|| {
        syn::Error::new(
            method_ident.span(),
            "method attribute requires path = \"/...\"",
        )
    })?;
    let method_value = method.value().to_ascii_uppercase();
    if !matches!(
        method_value.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    ) {
        return Err(syn::Error::new_spanned(
            method,
            format!("unsupported HTTP method {method_value}"),
        ));
    }
    let path_value = path.value();
    validate_route(&path_value, path.span())?;
    Ok(HttpMapping {
        method: method_value,
        path: path_value,
    })
}

fn validate_route(path: &str, span: proc_macro2::Span) -> syn::Result<BTreeSet<String>> {
    if !path.starts_with('/')
        || path.contains(['?', '#'])
        || path.contains("//")
        || (path.len() > 1 && path.ends_with('/'))
    {
        return Err(syn::Error::new(
            span,
            "HTTP paths must be absolute canonical route templates without query or fragment",
        ));
    }
    let mut names = BTreeSet::new();
    for segment in path
        .trim_matches('/')
        .split('/')
        .filter(|value| !value.is_empty())
    {
        if let Some(name) = segment
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        {
            validate_identity(name, "HTTP path parameter", span)?;
            if !names.insert(name.to_owned()) {
                return Err(syn::Error::new(span, "HTTP path parameters must be unique"));
            }
        } else if segment.contains(['{', '}']) {
            return Err(syn::Error::new(
                span,
                "HTTP path parameters must occupy a complete path segment",
            ));
        }
    }
    Ok(names)
}

fn is_rpc_attr(attribute: &Attribute) -> bool {
    attribute.path().is_ident("rpc")
}

pub(crate) fn is_param_attr(attribute: &Attribute) -> bool {
    attribute.path().is_ident("param")
}

fn direct_generic_type<'a>(kind: &'a Type, expected: &str) -> Option<&'a Type> {
    let Type::Path(TypePath {
        qself: None, path, ..
    }) = kind
    else {
        return None;
    };
    let segment = path.segments.last()?;
    if segment.ident != expected {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    if arguments.args.len() != 1 {
        return None;
    }
    match arguments.args.first()? {
        GenericArgument::Type(kind) => Some(kind),
        _ => None,
    }
}

fn is_json_primitive_type(kind: &Type) -> bool {
    let Type::Path(TypePath {
        qself: None, path, ..
    }) = kind
    else {
        return false;
    };
    path.segments.last().is_some_and(|segment| {
        matches!(
            segment.ident.to_string().as_str(),
            "bool"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "f32"
                | "f64"
        )
    })
}

fn route_shape(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.starts_with('{') {
                "{}"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn validate_identity(value: &str, field: &str, span: proc_macro2::Span) -> syn::Result<()> {
    let valid = !value.is_empty()
        && value.len() <= MAX_IDENTITY_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(syn::Error::new(
            span,
            format!("invalid {field}: expected 1-128 ASCII letters, digits, '.', '_' or '-'"),
        ))
    }
}

fn reject_conditional_attributes(attributes: &[Attribute], owner: &str) -> syn::Result<()> {
    if let Some(attribute) = attributes
        .iter()
        .find(|attribute| attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr"))
    {
        Err(syn::Error::new_spanned(
            attribute,
            format!("{owner} must not use conditional compilation attributes"),
        ))
    } else {
        Ok(())
    }
}

fn validate_owned_type(kind: &Type) -> syn::Result<()> {
    let mut validator = OwnedTypeValidator { error: None };
    validator.visit_type(kind);
    validator.error.map_or(Ok(()), Err)
}

struct OwnedTypeValidator {
    error: Option<syn::Error>,
}

impl OwnedTypeValidator {
    fn reject(&mut self, tokens: impl ToTokens, message: &'static str) {
        if self.error.is_none() {
            self.error = Some(syn::Error::new_spanned(tokens, message));
        }
    }
}

impl<'ast> Visit<'ast> for OwnedTypeValidator {
    fn visit_type_reference(&mut self, node: &'ast syn::TypeReference) {
        self.reject(node, "RPC request and response types must be owned");
    }

    fn visit_lifetime(&mut self, node: &'ast syn::Lifetime) {
        self.reject(node, "RPC types must not contain lifetime arguments");
    }

    fn visit_type_impl_trait(&mut self, node: &'ast syn::TypeImplTrait) {
        self.reject(node, "RPC types must not use impl Trait");
    }

    fn visit_type_infer(&mut self, node: &'ast syn::TypeInfer) {
        self.reject(node, "RPC types must not use inferred types");
    }

    fn visit_type_trait_object(&mut self, node: &'ast syn::TypeTraitObject) {
        self.reject(node, "RPC types must not use trait objects");
    }

    fn visit_type_path(&mut self, node: &'ast TypePath) {
        if node.qself.is_none()
            && node
                .path
                .segments
                .first()
                .is_some_and(|segment| segment.ident == "Self")
        {
            self.reject(node, "RPC types must not depend on Self");
            return;
        }
        syn::visit::visit_type_path(self, node);
    }
}
