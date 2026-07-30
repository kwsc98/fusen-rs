use crate::args::{MethodArgs, ServiceArgs, SpringArgs};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use syn::visit::Visit;
use syn::{
    Attribute, FnArg, GenericArgument, GenericParam, Generics, Ident, ItemTrait, Meta,
    PathArguments, ReturnType, Signature, TraitItem, Type, TypePath, WherePredicate,
};

const MAX_IDENTITY_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Idempotency {
    None,
    Idempotent,
    Safe,
}

impl Idempotency {
    fn parse(value: Option<&syn::LitStr>) -> syn::Result<Self> {
        let Some(value) = value else {
            return Ok(Self::None);
        };
        match value.value().as_str() {
            "none" => Ok(Self::None),
            "idempotent" => Ok(Self::Idempotent),
            "safe" => Ok(Self::Safe),
            _ => Err(syn::Error::new_spanned(
                value,
                "idempotency must be one of `none`, `idempotent`, or `safe`",
            )),
        }
    }
}

pub(crate) struct SpringMapping {
    pub(crate) method: String,
    pub(crate) path: String,
}

pub(crate) struct Method {
    pub(crate) ident: Ident,
    pub(crate) request: Type,
    pub(crate) response: Type,
    pub(crate) idempotency: Idempotency,
    pub(crate) spring: Option<SpringMapping>,
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
    let mut spring_routes = BTreeMap::<(String, String), Ident>::new();
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
        let (request, response) = validate_signature(&method.sig)?;
        validate_identity(
            method.sig.ident.to_string().trim_start_matches("r#"),
            "method name",
            method.sig.ident.span(),
        )?;
        let method_args = method_args(&method.attrs)?;
        let idempotency = Idempotency::parse(method_args.idempotency.as_ref())?;
        let spring = method_args
            .spring
            .map(|spring| validate_spring(spring, idempotency, &method.sig.ident))
            .transpose()?;
        if spring
            .as_ref()
            .is_some_and(|mapping| mapping.method == "HEAD")
            && !matches!(&response, Type::Tuple(tuple) if tuple.elems.is_empty())
        {
            return Err(syn::Error::new_spanned(
                &method.sig.output,
                "Spring HEAD mappings must return Result<RpcResponse<()>, RpcError>",
            ));
        }
        if let Some(spring) = &spring {
            let key = (spring.method.clone(), route_shape(&spring.path));
            if let Some(first) = spring_routes.insert(key, method.sig.ident.clone()) {
                return Err(syn::Error::new(
                    method.sig.ident.span(),
                    format!(
                        "duplicate Spring route {} {}; it conflicts with method `{first}`",
                        spring.method, spring.path
                    ),
                ));
            }
        }
        methods.push(Method {
            ident: method.sig.ident.clone(),
            request,
            response,
            idempotency,
            spring,
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

fn method_args(attributes: &[Attribute]) -> syn::Result<MethodArgs> {
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
    Ok(parsed.unwrap_or_default())
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

fn validate_signature(signature: &Signature) -> syn::Result<(Type, Type)> {
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
    if signature.inputs.len() != 2 {
        return Err(syn::Error::new_spanned(
            &signature.inputs,
            "RPC methods must receive exactly one RpcRequest<T> after &self",
        ));
    }
    let FnArg::Typed(request) = &signature.inputs[1] else {
        unreachable!("the second input cannot be a receiver")
    };
    let request = wrapper_inner(&request.ty, "RpcRequest", "request parameter")?;
    validate_owned_type(&request)?;

    let ReturnType::Type(_, output) = &signature.output else {
        return Err(syn::Error::new_spanned(
            &signature.output,
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
    Ok((request, response))
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

fn validate_spring(
    args: SpringArgs,
    idempotency: Idempotency,
    method_ident: &Ident,
) -> syn::Result<SpringMapping> {
    let method = args.method.ok_or_else(|| {
        syn::Error::new(
            method_ident.span(),
            "Spring mapping requires method = \"...\"",
        )
    })?;
    let path = args.path.ok_or_else(|| {
        syn::Error::new(
            method_ident.span(),
            "Spring mapping requires path = \"/...\"",
        )
    })?;
    let method_value = method.value().to_ascii_uppercase();
    if !matches!(
        method_value.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    ) {
        return Err(syn::Error::new_spanned(
            method,
            format!("unsupported Spring HTTP method {method_value}"),
        ));
    }
    match idempotency {
        Idempotency::Safe if !matches!(method_value.as_str(), "GET" | "HEAD") => {
            return Err(syn::Error::new_spanned(
                method,
                "safe methods may use only GET or HEAD Spring mappings",
            ));
        }
        Idempotency::Idempotent
            if !matches!(
                method_value.as_str(),
                "GET" | "HEAD" | "PUT" | "DELETE" | "POST"
            ) =>
        {
            return Err(syn::Error::new_spanned(
                method,
                "idempotent Spring mappings may use GET, HEAD, PUT, DELETE, or an explicitly declared POST",
            ));
        }
        _ => {}
    }
    let path_value = path.value();
    validate_route(&path_value, path.span())?;
    Ok(SpringMapping {
        method: method_value,
        path: path_value,
    })
}

fn validate_route(path: &str, span: proc_macro2::Span) -> syn::Result<()> {
    if !path.starts_with('/')
        || path.contains(['?', '#'])
        || path.contains("//")
        || (path.len() > 1 && path.ends_with('/'))
    {
        return Err(syn::Error::new(
            span,
            "Spring paths must be absolute canonical route templates without query or fragment",
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
            validate_identity(name, "Spring path parameter", span)?;
            if !names.insert(name.to_owned()) {
                return Err(syn::Error::new(
                    span,
                    "Spring path parameters must be unique",
                ));
            }
        } else if segment.contains(['{', '}']) {
            return Err(syn::Error::new(
                span,
                "Spring path parameters must occupy a complete path segment",
            ));
        }
    }
    Ok(())
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
