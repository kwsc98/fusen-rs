use crate::args::{MethodArgs, ServiceArgs, SpringArgs};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use syn::visit::Visit;
use syn::{
    Attribute, FnArg, GenericArgument, GenericParam, Generics, Ident, ItemTrait, LitStr, Meta, Pat,
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
    fn parse(value: Option<&LitStr>) -> syn::Result<Self> {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParameterSource {
    Path,
    Query,
    Body,
}

#[derive(Clone)]
pub(crate) struct Parameter {
    pub(crate) ident: Ident,
    pub(crate) kind: Type,
    pub(crate) wire_name: String,
    pub(crate) spring_source: Option<ParameterSource>,
}

pub(crate) struct SpringMapping {
    pub(crate) method: String,
    pub(crate) path: String,
}

pub(crate) struct Method {
    pub(crate) ident: Ident,
    pub(crate) declared_result: Type,
    pub(crate) success: Type,
    pub(crate) parameters: Vec<Parameter>,
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
            "RPC traits must be ordinary safe traits",
        ));
    }
    validate_trait_generics(&item.generics)?;
    reject_conditional_attributes(&item.attrs, "RPC service traits")?;

    let name = args.name.ok_or_else(|| {
        syn::Error::new(
            item.ident.span(),
            "service name is required: use `#[service(name = \"...\")]`",
        )
    })?;
    validate_identity(&name.value(), "service name", name.span())?;
    if let Some(group) = &args.group {
        validate_identity(&group.value(), "service group", group.span())?;
    }
    if let Some(version) = &args.version {
        validate_identity(&version.value(), "service version", version.span())?;
    }

    if item.items.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.ident,
            "RPC traits must declare at least one method",
        ));
    }
    if item.items.len() > usize::from(u16::MAX) + 1 {
        return Err(syn::Error::new_spanned(
            &item.ident,
            "RPC traits may declare at most 65536 methods",
        ));
    }

    let mut methods = Vec::with_capacity(item.items.len());
    let mut spring_routes = BTreeMap::<(String, String), Ident>::new();
    for trait_item in &item.items {
        let TraitItem::Fn(method) = trait_item else {
            return Err(syn::Error::new_spanned(
                trait_item,
                "RPC traits may contain only async methods",
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

        let args = method_args(&method.attrs)?;
        let idempotency = Idempotency::parse(args.idempotency.as_ref())?;
        let mut parameters = parameters(&method.sig)?;
        let (declared_result, success) = result_types(&method.sig.output)?;
        validate_owned_type(&success)?;
        let spring = args
            .spring
            .map(|spring| validate_spring(spring, idempotency, &mut parameters, &method.sig.ident))
            .transpose()?;
        if spring
            .as_ref()
            .is_some_and(|mapping| mapping.method == "HEAD")
            && !matches!(&success, Type::Tuple(tuple) if tuple.elems.is_empty())
        {
            return Err(syn::Error::new_spanned(
                &method.sig.output,
                "Spring HEAD mappings must return `Result<(), RpcError>` because HEAD responses have no body",
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
            declared_result,
            success,
            parameters,
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
            "RPC traits must not declare generic parameters",
        ));
    }
    if let Some(where_clause) = &generics.where_clause {
        for predicate in &where_clause.predicates {
            let WherePredicate::Type(predicate) = predicate else {
                return Err(syn::Error::new_spanned(
                    predicate,
                    "RPC trait where clauses may constrain only `Self`",
                ));
            };
            let Type::Path(TypePath {
                qself: None, path, ..
            }) = &predicate.bounded_ty
            else {
                return Err(syn::Error::new_spanned(
                    predicate,
                    "RPC trait where clauses may constrain only `Self`",
                ));
            };
            if !path.is_ident("Self") {
                return Err(syn::Error::new_spanned(
                    predicate,
                    "RPC trait where clauses may constrain only `Self`",
                ));
            }
        }
    }
    Ok(())
}

fn validate_signature(signature: &Signature) -> syn::Result<()> {
    if signature.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            signature,
            "RPC methods must be async",
        ));
    }
    if signature.constness.is_some()
        || matches!(signature.safety, syn::Safety::Unsafe(_))
        || signature.abi.is_some()
        || signature.variadic.is_some()
    {
        return Err(syn::Error::new_spanned(
            signature,
            "RPC methods must be ordinary safe, non-const Rust methods",
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
            "RPC methods must have an immutable `&self` receiver",
        ));
    };
    let valid_receiver = matches!(&receiver.kind, syn::ReceiverKind::Reference(_, None, None))
        && receiver.mutability.is_none();
    if !valid_receiver {
        return Err(syn::Error::new_spanned(
            receiver,
            "RPC methods must have an immutable `&self` receiver without an explicit lifetime",
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

fn parameters(signature: &Signature) -> syn::Result<Vec<Parameter>> {
    signature
        .inputs
        .iter()
        .skip(1)
        .map(|input| {
            let FnArg::Typed(input) = input else {
                unreachable!("the signature validator rejected extra receivers")
            };
            let Pat::Ident(pattern) = input.pat.as_ref() else {
                return Err(syn::Error::new_spanned(
                    &input.pat,
                    "RPC parameters must use plain immutable identifier patterns",
                ));
            };
            if pattern.by_ref.is_some() || pattern.mutability.is_some() || pattern.subpat.is_some()
            {
                return Err(syn::Error::new_spanned(
                    pattern,
                    "RPC parameters must use plain immutable identifier patterns",
                ));
            }
            let wire_name = pattern
                .ident
                .to_string()
                .trim_start_matches("r#")
                .to_owned();
            validate_identity(&wire_name, "parameter name", pattern.ident.span())?;
            Ok(Parameter {
                ident: pattern.ident.clone(),
                kind: input.ty.as_ref().clone(),
                wire_name,
                spring_source: None,
            })
        })
        .collect()
}

fn result_types(output: &ReturnType) -> syn::Result<(Type, Type)> {
    let ReturnType::Type(_, output) = output else {
        return Err(syn::Error::new_spanned(
            output,
            "RPC methods must explicitly return `Result<T, RpcError>`",
        ));
    };
    let Type::Path(result) = output.as_ref() else {
        return Err(syn::Error::new_spanned(
            output,
            "RPC methods must explicitly return `Result<T, RpcError>`",
        ));
    };
    let Some(segment) = result.path.segments.last() else {
        return Err(syn::Error::new_spanned(
            output,
            "RPC methods must explicitly return `Result<T, RpcError>`",
        ));
    };
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Err(syn::Error::new_spanned(
            output,
            "RPC methods must explicitly return `Result<T, RpcError>`",
        ));
    };
    if segment.ident != "Result" || arguments.args.len() != 2 {
        return Err(syn::Error::new_spanned(
            output,
            "RPC methods must explicitly return `Result<T, RpcError>`",
        ));
    }
    let mut arguments = arguments.args.iter();
    let Some(GenericArgument::Type(success)) = arguments.next() else {
        return Err(syn::Error::new_spanned(
            output,
            "RPC methods must explicitly return `Result<T, RpcError>`",
        ));
    };
    let Some(GenericArgument::Type(error)) = arguments.next() else {
        return Err(syn::Error::new_spanned(
            output,
            "RPC methods must explicitly return `Result<T, RpcError>`",
        ));
    };
    let Type::Path(error) = error else {
        return Err(syn::Error::new_spanned(
            error,
            "the RPC error type must be `RpcError`",
        ));
    };
    if error
        .path
        .segments
        .last()
        .is_none_or(|segment| segment.ident != "RpcError" || !segment.arguments.is_empty())
    {
        return Err(syn::Error::new_spanned(
            error,
            "the RPC error type must be `RpcError`",
        ));
    }
    Ok((output.as_ref().clone(), success.clone()))
}

fn validate_spring(
    args: SpringArgs,
    idempotency: Idempotency,
    parameters: &mut [Parameter],
    method_ident: &Ident,
) -> syn::Result<SpringMapping> {
    let method = args.method.ok_or_else(|| {
        syn::Error::new(
            method_ident.span(),
            "Spring mapping requires `method = \"...\"`",
        )
    })?;
    let path = args.path.ok_or_else(|| {
        syn::Error::new(
            method_ident.span(),
            "Spring mapping requires `path = \"/...\"`",
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
    let placeholders = validate_route(&path_value, path.span())?;
    let parameter_names = parameters
        .iter()
        .map(|parameter| parameter.wire_name.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(unknown) = placeholders
        .iter()
        .find(|placeholder| !parameter_names.contains(placeholder.as_str()))
    {
        return Err(syn::Error::new_spanned(
            &path,
            format!("path parameter `{unknown}` has no matching method parameter"),
        ));
    }

    let mut declared = BTreeMap::<String, (ParameterSource, proc_macro2::Span)>::new();
    for name in &placeholders {
        declared.insert(name.clone(), (ParameterSource::Path, path.span()));
    }
    for query in args.query {
        insert_parameter_source(&mut declared, &query, ParameterSource::Query)?;
    }
    if let Some(body) = args.body {
        insert_parameter_source(&mut declared, &body, ParameterSource::Body)?;
    }
    if let Some((unknown, (_, span))) = declared
        .iter()
        .find(|(name, _)| !parameter_names.contains(name.as_str()))
    {
        return Err(syn::Error::new(
            *span,
            format!("Spring parameter `{unknown}` has no matching method parameter"),
        ));
    }
    if let Some(parameter) = parameters
        .iter()
        .find(|parameter| !declared.contains_key(&parameter.wire_name))
    {
        return Err(syn::Error::new(
            parameter.ident.span(),
            format!(
                "Spring source for parameter `{}` is missing; list it in `query = [...]` or `body = \"...\"`",
                parameter.wire_name
            ),
        ));
    }
    for parameter in parameters {
        parameter.spring_source = declared
            .get(&parameter.wire_name)
            .map(|(source, _)| *source);
    }

    Ok(SpringMapping {
        method: method_value,
        path: path_value,
    })
}

fn insert_parameter_source(
    declared: &mut BTreeMap<String, (ParameterSource, proc_macro2::Span)>,
    name: &LitStr,
    source: ParameterSource,
) -> syn::Result<()> {
    validate_identity(&name.value(), "Spring parameter name", name.span())?;
    if let Some((previous, _)) = declared.insert(name.value(), (source, name.span())) {
        return Err(syn::Error::new_spanned(
            name,
            format!(
                "Spring parameter `{}` is declared more than once (already mapped as {})",
                name.value(),
                source_name(previous)
            ),
        ));
    }
    Ok(())
}

fn source_name(source: ParameterSource) -> &'static str {
    match source {
        ParameterSource::Path => "path",
        ParameterSource::Query => "query",
        ParameterSource::Body => "body",
    }
}

fn validate_route(path: &str, span: proc_macro2::Span) -> syn::Result<BTreeSet<String>> {
    if !path.starts_with('/')
        || path.contains(['?', '#'])
        || path.contains("//")
        || (path.len() > 1 && path.ends_with('/'))
    {
        return Err(syn::Error::new(
            span,
            "Spring paths must be absolute, canonical route templates without query or fragment",
        ));
    }
    let mut names = BTreeSet::new();
    for segment in path
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
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
    Ok(names)
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
        self.reject(node, "RPC parameter and success types must be owned");
    }

    fn visit_lifetime(&mut self, node: &'ast syn::Lifetime) {
        self.reject(node, "RPC types must not contain lifetime arguments");
    }

    fn visit_type_impl_trait(&mut self, node: &'ast syn::TypeImplTrait) {
        self.reject(node, "RPC types must not use `impl Trait`");
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
            self.reject(node, "RPC types must not depend on `Self`");
            return;
        }
        syn::visit::visit_type_path(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn validate_trait(
        args: proc_macro2::TokenStream,
        item: proc_macro2::TokenStream,
    ) -> syn::Result<Service> {
        validate(syn::parse2(args)?, &syn::parse2(item)?)
    }

    #[test]
    fn infers_paths_and_requires_other_spring_sources() {
        let service = validate_trait(
            quote!(name = "user"),
            quote! {
                trait User {
                    #[method(
                        idempotency = "safe",
                        spring(method = "GET", path = "/users/{id}", query = ["expand"])
                    )]
                    async fn get(&self, id: String, expand: Option<bool>) -> Result<User, RpcError>;
                }
            },
        )
        .unwrap();
        assert_eq!(
            service.methods[0].parameters[0].spring_source,
            Some(ParameterSource::Path)
        );
        assert_eq!(
            service.methods[0].parameters[1].spring_source,
            Some(ParameterSource::Query)
        );

        let missing = validate_trait(
            quote!(name = "user"),
            quote! {
                trait User {
                    #[method(spring(method = "POST", path = "/users"))]
                    async fn create(&self, request: Request) -> Result<User, RpcError>;
                }
            },
        );
        assert!(missing.is_err());
    }

    #[test]
    fn rejects_wrong_error_and_route_collision() {
        let wrong_error = validate_trait(
            quote!(name = "user"),
            quote! {
                trait User {
                    async fn get(&self) -> Result<User, String>;
                }
            },
        );
        assert!(wrong_error.is_err());

        let duplicate = validate_trait(
            quote!(name = "user"),
            quote! {
                trait User {
                    #[method(spring(method = "GET", path = "/users/{id}"))]
                    async fn by_id(&self, id: String) -> Result<User, RpcError>;
                    #[method(spring(method = "GET", path = "/users/{name}"))]
                    async fn by_name(&self, name: String) -> Result<User, RpcError>;
                }
            },
        );
        assert!(duplicate.is_err());
    }

    #[test]
    fn keeps_idempotency_explicit_and_accepts_declared_idempotent_post() {
        let service = validate_trait(
            quote!(name = "user"),
            quote! {
                trait User {
                    async fn lookup(&self) -> Result<User, RpcError>;

                    #[method(
                        idempotency = "idempotent",
                        spring(method = "POST", path = "/users", body = "request")
                    )]
                    async fn upsert(&self, request: Request) -> Result<User, RpcError>;
                }
            },
        )
        .unwrap();

        assert_eq!(service.methods[0].idempotency, Idempotency::None);
        assert_eq!(service.methods[1].idempotency, Idempotency::Idempotent);
        assert_eq!(
            service.methods[1]
                .spring
                .as_ref()
                .map(|mapping| mapping.method.as_str()),
            Some("POST")
        );
    }

    #[test]
    fn head_mappings_require_a_unit_success_type() {
        let invalid = validate_trait(
            quote!(name = "health"),
            quote! {
                trait Health {
                    #[method(idempotency = "safe", spring(method = "HEAD", path = "/health"))]
                    async fn health(&self) -> Result<String, RpcError>;
                }
            },
        );
        assert!(invalid.is_err());

        validate_trait(
            quote!(name = "health"),
            quote! {
                trait Health {
                    #[method(idempotency = "safe", spring(method = "HEAD", path = "/health"))]
                    async fn health(&self) -> Result<(), RpcError>;
                }
            },
        )
        .unwrap();
    }
}
