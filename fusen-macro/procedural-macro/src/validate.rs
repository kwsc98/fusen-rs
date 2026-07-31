use crate::{
    args::{MethodArgs, ServiceArgs},
    sensitive::{SensitiveOverride, parse_sensitive_attrs},
};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use syn::spanned::Spanned;
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
    pub(crate) spring_text: bool,
    pub(crate) sensitivity: Option<SensitiveOverride>,
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
    let path = attribute.path();
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
    matches!(segments.as_slice(), [method] if method == "method")
        || matches!(segments.as_slice(), [owner, method]
            if method == "method"
                && (owner == &crate::runtime_crate_name()
                    || owner == &crate::procedural_macro_crate_name()))
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
    for input in &signature.inputs {
        match input {
            FnArg::Receiver(receiver) => {
                reject_conditional_attributes(&receiver.attrs, "RPC method receivers")?;
            }
            FnArg::Typed(input) => {
                reject_conditional_attributes(&input.attrs, "RPC method parameters")?;
            }
        }
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
    let mut names = BTreeMap::<String, proc_macro2::Span>::new();
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

        let sensitivity = parse_sensitive_attrs(&input.attrs)?;

        let mut explicit_source = None;
        let mut wire_name: Option<syn::LitStr> = None;
        let mut repeated: Option<proc_macro2::Span> = None;
        for attribute in input
            .attrs
            .iter()
            .filter(|attribute| is_param_attr(attribute))
        {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("context")
                    || meta.path.is_ident("path")
                    || meta.path.is_ident("query")
                    || meta.path.is_ident("body")
                {
                    let source = if meta.path.is_ident("context") {
                        ParameterSource::Context
                    } else if meta.path.is_ident("path") {
                        ParameterSource::Path
                    } else if meta.path.is_ident("query") {
                        ParameterSource::Query
                    } else {
                        ParameterSource::Body
                    };
                    if meta.input.peek(syn::Token![=])
                        || meta.input.peek(syn::token::Paren)
                    {
                        return Err(meta.error(format!(
                            "`{}` does not accept a value",
                            source_name(source)
                        )));
                    }
                    if let Some((first_source, first_span)) = explicit_source {
                        let message = if first_source == source {
                            format!("duplicate parameter source `{}`", source_name(source))
                        } else {
                            format!(
                                "conflicting parameter sources `{}` and `{}`",
                                source_name(first_source),
                                source_name(source),
                            )
                        };
                        let mut error = meta.error(message);
                        error.combine(syn::Error::new(first_span, "first source declared here"));
                        return Err(error);
                    }
                    explicit_source = Some((source, meta.path.span()));
                    Ok(())
                } else if meta.path.is_ident("name") {
                    if let Some(first) = &wire_name {
                        let mut error = meta.error("duplicate parameter wire name");
                        error.combine(syn::Error::new(first.span(), "first wire name declared here"));
                        return Err(error);
                    }
                    wire_name = Some(meta.value()?.parse::<syn::LitStr>()?);
                    Ok(())
                } else if meta.path.is_ident("repeated") {
                    if meta.input.peek(syn::Token![=])
                        || meta.input.peek(syn::token::Paren)
                    {
                        return Err(meta.error("`repeated` does not accept a value"));
                    }
                    if let Some(first_span) = repeated {
                        let mut error = meta.error("duplicate parameter flag `repeated`");
                        error.combine(syn::Error::new(first_span, "first `repeated` declared here"));
                        return Err(error);
                    }
                    repeated = Some(meta.path.span());
                    Ok(())
                } else {
                    Err(meta.error(
                        "unknown parameter field; expected context, path, query, body, name, or repeated",
                    ))
                }
            })?;
        }
        let rust_name = pattern
            .ident
            .to_string()
            .trim_start_matches("r#")
            .to_owned();

        if explicit_source.is_some_and(|(source, _)| source == ParameterSource::Context) {
            if sensitivity.is_some() {
                return Err(syn::Error::new_spanned(
                    input,
                    "#[param(context)] parameters cannot declare sensitivity metadata",
                ));
            }
            if wire_name.is_some() {
                return Err(syn::Error::new_spanned(
                    input,
                    "#[param(context)] parameters must not declare a wire name",
                ));
            }
            if let Some(span) = repeated {
                return Err(syn::Error::new(
                    span,
                    "#[param(context)] parameters cannot be repeated",
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
                spring_text: false,
                sensitivity: None,
            });
            continue;
        }

        let (wire_name, wire_name_span) = wire_name.map_or_else(
            || (rust_name.clone(), pattern.ident.span()),
            |name| (name.value(), name.span()),
        );
        validate_identity(&wire_name, "RPC parameter name", wire_name_span)?;
        if let Some(first_span) = names.insert(wire_name.clone(), wire_name_span) {
            let mut error = syn::Error::new(
                wire_name_span,
                format!("duplicate RPC wire parameter name `{wire_name}`"),
            );
            error.combine(syn::Error::new(first_span, "first wire name declared here"));
            return Err(error);
        }

        let source = explicit_source
            .map(|(source, _)| source)
            .unwrap_or_else(|| {
                if placeholders.contains(&wire_name) {
                    ParameterSource::Path
                } else if body_fields_by_default {
                    ParameterSource::BodyField
                } else {
                    ParameterSource::Query
                }
            });
        if source != ParameterSource::Path && placeholders.contains(&wire_name) {
            let span = explicit_source
                .map(|(_, span)| span)
                .unwrap_or_else(|| pattern.ident.span());
            return Err(syn::Error::new(
                span,
                format!(
                    "HTTP path placeholder `{{{wire_name}}}` requires this parameter to use `#[param(path)]`"
                ),
            ));
        }
        if let Some(span) = repeated
            && !explicit_source.is_some_and(|(source, _)| source == ParameterSource::Query)
        {
            return Err(syn::Error::new(
                span,
                "repeated query parameters must use `#[param(query, repeated)]`",
            ));
        }
        if source == ParameterSource::Path && !placeholders.contains(&wire_name) {
            return Err(syn::Error::new_spanned(
                input,
                format!(
                    "#[param(path)] parameter `{wire_name}` requires a matching `{{{wire_name}}}` path placeholder"
                ),
            ));
        }
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
            if direct_standard_generic_type(&input.ty, "Option")
                .and_then(|inner| direct_standard_generic_type(inner, "Vec"))
                .is_some()
            {
                return Err(syn::Error::new_spanned(
                    &input.ty,
                    "query parameters may not use Option<Vec<T>>; use #[param(query, repeated)] Vec<T> so omission has one unambiguous empty-list meaning",
                ));
            }
            if let Some(element) = direct_standard_generic_type(&input.ty, "Vec") {
                if repeated.is_none() {
                    return Err(syn::Error::new_spanned(
                        &input.ty,
                        "Vec query parameters must declare #[param(query, repeated)]",
                    ));
                }
                if direct_standard_generic_type(element, "Option").is_some()
                    || direct_standard_generic_type(element, "Vec").is_some()
                {
                    return Err(syn::Error::new_spanned(
                        element,
                        "repeated query elements must be scalar values, not Option<T> or Vec<T>",
                    ));
                }
            }
            repeated.is_some()
        } else {
            if source == ParameterSource::Path
                && (direct_standard_generic_type(&input.ty, "Option").is_some()
                    || direct_standard_generic_type(&input.ty, "Vec").is_some())
            {
                return Err(syn::Error::new_spanned(
                    &input.ty,
                    "path parameters must be required scalar values",
                ));
            }
            false
        };
        parameters.push(Parameter {
            ident: pattern.ident.clone(),
            kind: input.ty.as_ref().clone(),
            wire_name,
            source,
            repeated,
            spring_text: matches!(source, ParameterSource::Path | ParameterSource::Query),
            sensitivity,
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
    if result.qself.is_some() || !is_standard_result_path(&result.path) || arguments.args.len() != 2
    {
        return Err(syn::Error::new_spanned(
            output,
            "RPC methods must return Result<RpcResponse<T>, RpcError>",
        ));
    }
    let mut args = arguments.args.iter();
    let Some(success) = args.next() else {
        unreachable!("the Result arity was checked")
    };
    let GenericArgument::Type(success) = success else {
        return Err(syn::Error::new_spanned(
            success,
            "RPC Result success argument must be a response type",
        ));
    };
    let response = runtime_wrapper_inner(success)?;
    validate_owned_type(&response)?;
    let Some(GenericArgument::Type(Type::Path(error))) = args.next() else {
        return Err(syn::Error::new_spanned(
            output,
            "RPC methods must use RpcError as their error type",
        ));
    };
    if error.qself.is_some() || !is_runtime_type_path(&error.path, "RpcError") {
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

fn runtime_wrapper_inner(kind: &Type) -> syn::Result<Type> {
    let Type::Path(TypePath {
        qself: None, path, ..
    }) = kind
    else {
        return Err(syn::Error::new_spanned(
            kind,
            "RPC success type must be the fusen-rs RpcResponse<T>",
        ));
    };
    let Some(segment) = path.segments.last() else {
        return Err(syn::Error::new_spanned(
            kind,
            "RPC success type must be the fusen-rs RpcResponse<T>",
        ));
    };
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Err(syn::Error::new_spanned(
            kind,
            "RPC success type must be the fusen-rs RpcResponse<T>",
        ));
    };
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    let valid_path = matches!(segments.as_slice(), [response] if response == "RpcResponse")
        || matches!(segments.as_slice(), [runtime, response]
            if runtime == &crate::runtime_crate_name() && response == "RpcResponse");
    if !valid_path
        || path
            .segments
            .iter()
            .take(path.segments.len().saturating_sub(1))
            .any(|segment| !segment.arguments.is_empty())
        || arguments.args.len() != 1
    {
        return Err(syn::Error::new_spanned(
            kind,
            "RPC success type must be the fusen-rs RpcResponse<T>",
        ));
    }
    match arguments.args.first() {
        Some(GenericArgument::Type(kind)) => Ok(kind.clone()),
        _ => Err(syn::Error::new_spanned(
            kind,
            "RPC success type must be the fusen-rs RpcResponse<T>",
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
    if !path.starts_with('/') {
        return Err(syn::Error::new(
            span,
            "HTTP route templates must be absolute paths starting with '/'",
        ));
    }
    if path.contains(['?', '#']) {
        return Err(syn::Error::new(
            span,
            "HTTP route templates must not contain a query or fragment",
        ));
    }
    if path.contains("//") || (path.len() > 1 && path.ends_with('/')) {
        return Err(syn::Error::new(
            span,
            "HTTP route templates must not contain empty or trailing segments",
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
        } else {
            validate_route_literal(segment, span)?;
        }
    }
    Ok(names)
}

fn validate_route_literal(segment: &str, span: proc_macro2::Span) -> syn::Result<()> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' {
            let Some((&high, &low)) = bytes.get(index + 1).zip(bytes.get(index + 2)) else {
                return Err(syn::Error::new(
                    span,
                    "HTTP route percent escapes must use uppercase `%HH` syntax",
                ));
            };
            if !high.is_ascii_hexdigit() || !low.is_ascii_hexdigit() {
                return Err(syn::Error::new(
                    span,
                    "HTTP route percent escapes must use uppercase `%HH` syntax",
                ));
            }
            if high.is_ascii_lowercase() || low.is_ascii_lowercase() {
                return Err(syn::Error::new(
                    span,
                    "HTTP route percent escapes must use uppercase hexadecimal digits",
                ));
            }
            let decoded_byte = (hex_value(high) << 4) | hex_value(low);
            if decoded_byte.is_ascii() {
                return Err(syn::Error::new(
                    span,
                    "ASCII HTTP path characters must not be percent-encoded",
                ));
            }
            decoded.push(decoded_byte);
            index += 3;
            continue;
        }
        if !is_rfc3986_pchar(byte) {
            return Err(syn::Error::new(
                span,
                "HTTP route literals must use ASCII RFC 3986 path characters; encode non-ASCII text as uppercase UTF-8 percent escapes",
            ));
        }
        decoded.push(byte);
        index += 1;
    }

    let decoded = std::str::from_utf8(&decoded).map_err(|_| {
        syn::Error::new(span, "percent-encoded HTTP route text must be valid UTF-8")
    })?;
    if decoded.chars().any(char::is_whitespace) || decoded.chars().any(char::is_control) {
        return Err(syn::Error::new(
            span,
            "HTTP route literals must not contain whitespace or control characters",
        ));
    }
    if matches!(decoded, "." | "..") {
        return Err(syn::Error::new(
            span,
            "HTTP route templates must not contain '.' or '..' segments",
        ));
    }
    Ok(())
}

const fn is_rfc3986_pchar(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b':'
                | b'@'
        )
}

const fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
}

fn is_rpc_attr(attribute: &Attribute) -> bool {
    attribute.path().is_ident("rpc")
}

pub(crate) fn is_param_attr(attribute: &Attribute) -> bool {
    attribute.path().is_ident("param")
}

fn direct_standard_generic_type<'a>(kind: &'a Type, expected: &str) -> Option<&'a Type> {
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
    let prefix = path
        .segments
        .iter()
        .take(path.segments.len().saturating_sub(1))
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    let valid_prefix = prefix.is_empty()
        || matches!((expected, prefix.as_slice()), ("Vec", [root, module])
            if matches!(root.as_str(), "std" | "alloc") && module == "vec")
        || matches!((expected, prefix.as_slice()), ("Option", [root, module])
            if matches!(root.as_str(), "std" | "core") && module == "option");
    if !valid_prefix
        || path
            .segments
            .iter()
            .take(path.segments.len().saturating_sub(1))
            .any(|segment| !segment.arguments.is_empty())
    {
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

const fn source_name(source: ParameterSource) -> &'static str {
    match source {
        ParameterSource::Context => "context",
        ParameterSource::Path => "path",
        ParameterSource::Query => "query",
        ParameterSource::BodyField => "body field",
        ParameterSource::Body => "body",
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_route_literals() {
        for path in ["/", "/users/{id}", "/a-._~!$&'()*+,;=:@/caf%C3%A9"] {
            assert!(
                validate_route(path, proc_macro2::Span::call_site()).is_ok(),
                "{path}"
            );
        }
    }

    #[test]
    fn rejects_noncanonical_route_literals() {
        for path in [
            "relative",
            "/users/",
            "/users//active",
            "/users?active=true",
            "/用户",
            "/white space",
            "/%e7%94%a8",
            "/%41",
            "/%FF",
            "/%C2%A0",
            "/%C2%85",
            "/.",
            "/..",
            "/back\\slash",
        ] {
            assert!(
                validate_route(path, proc_macro2::Span::call_site()).is_err(),
                "{path}"
            );
        }
    }
}
