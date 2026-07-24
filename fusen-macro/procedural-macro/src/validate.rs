use crate::get_asset_by_attrs;
use quote::ToTokens;
use std::collections::BTreeSet;
use syn::{
    FnArg, GenericParam, Generics, ItemTrait, Pat, ReturnType, Signature, TraitItem, Type,
    TypePath, WherePredicate, parse_quote, visit::Visit,
};

#[derive(Clone, Copy)]
pub(crate) enum ParameterSource {
    Path,
    Query,
    Body,
}

pub(crate) struct MethodResource {
    pub(crate) name: String,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) parameters: Vec<(String, ParameterSource)>,
}

pub(crate) fn validate_trait(item: &ItemTrait) -> Result<Vec<MethodResource>, syn::Error> {
    if item.unsafety.is_some() || item.modifiers.auto_token.is_some() {
        return Err(syn::Error::new_spanned(
            &item.ident,
            "RPC traits must be ordinary safe traits",
        ));
    }
    validate_trait_generics(&item.generics)?;
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
    if item.items.iter().any(
        |item| matches!(item, TraitItem::Fn(method) if method.sig.ident == "__fusen_service_descriptor"),
    ) {
        return Err(syn::Error::new_spanned(
            &item.ident,
            "__fusen_service_descriptor is reserved by fusen_trait",
        ));
    }

    let parent = get_asset_by_attrs(&item.attrs)?;
    let parent_path = parent.path.unwrap_or_else(|| format!("/{}", item.ident));
    let parent_method = normalize_method(parent.method.as_deref().unwrap_or("POST"));
    validate_method(&parent_method, item.ident.span())?;
    let mut resources = Vec::with_capacity(item.items.len());
    for trait_item in &item.items {
        let TraitItem::Fn(method) = trait_item else {
            return Err(syn::Error::new_spanned(
                trait_item,
                "RPC traits may contain only async methods",
            ));
        };
        validate_signature(&method.sig)?;
        if method.default.is_some() {
            return Err(syn::Error::new_spanned(
                method,
                "RPC methods must not provide default implementations",
            ));
        }

        let resource = get_asset_by_attrs(&method.attrs)?;
        let verb = normalize_method(resource.method.as_deref().unwrap_or(&parent_method));
        validate_method(&verb, method.sig.ident.span())?;
        let child_path = resource
            .path
            .unwrap_or_else(|| format!("/{}", method.sig.ident));
        let path = join_paths(&parent_path, &child_path);
        validate_route(&path, method.sig.ident.span())?;
        let placeholders = placeholders(&path, method.sig.ident.span())?;
        let mut parameter_names = BTreeSet::new();
        let mut parameters = Vec::new();
        for input in &method.sig.inputs {
            let FnArg::Typed(input) = input else {
                continue;
            };
            let Pat::Ident(pattern) = input.pat.as_ref() else {
                return Err(syn::Error::new_spanned(
                    &input.pat,
                    "RPC parameters must use identifier patterns",
                ));
            };
            if pattern.by_ref.is_some() || pattern.mutability.is_some() || pattern.subpat.is_some()
            {
                return Err(syn::Error::new_spanned(
                    pattern,
                    "RPC parameters must use plain immutable identifiers",
                ));
            }
            let name = pattern
                .ident
                .to_string()
                .trim_start_matches("r#")
                .to_owned();
            if !parameter_names.insert(name.clone()) {
                return Err(syn::Error::new_spanned(
                    pattern,
                    format!("duplicate RPC parameter `{name}`"),
                ));
            }
            let source = if placeholders.contains(&name) {
                ParameterSource::Path
            } else if matches!(verb.as_str(), "GET" | "DELETE" | "HEAD") {
                ParameterSource::Query
            } else {
                ParameterSource::Body
            };
            parameters.push((name, source));
        }
        if let Some(unknown) = placeholders.difference(&parameter_names).next() {
            return Err(syn::Error::new(
                method.sig.ident.span(),
                format!("route parameter {unknown} has no matching method parameter"),
            ));
        }
        resources.push(MethodResource {
            name: method.sig.ident.to_string(),
            method: verb,
            path,
            parameters,
        });
    }
    Ok(resources)
}

fn validate_trait_generics(generics: &Generics) -> Result<(), syn::Error> {
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

pub(crate) fn validate_signature(signature: &Signature) -> Result<(), syn::Error> {
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
    if let ReturnType::Type(_, output) = &signature.output {
        validate_owned_type(output)?;
    }
    Ok(())
}

pub(crate) fn validate_owned_type(kind: &Type) -> Result<(), syn::Error> {
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
        self.reject(node, "RPC parameter and return types must be owned");
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

pub(crate) fn output_type(output: &ReturnType) -> Type {
    match output {
        ReturnType::Default => parse_quote!(()),
        ReturnType::Type(_, output) => output.as_ref().clone(),
    }
}

fn placeholders(path: &str, span: proc_macro2::Span) -> Result<BTreeSet<String>, syn::Error> {
    let mut names = BTreeSet::new();
    for segment in path.trim_matches('/').split('/') {
        if let Some(name) = segment
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        {
            if name.is_empty() || !names.insert(name.to_owned()) {
                return Err(syn::Error::new(
                    span,
                    "route parameters must be non-empty and unique",
                ));
            }
        } else if segment.contains('{') || segment.contains('}') {
            return Err(syn::Error::new(
                span,
                "route parameters must occupy a full segment",
            ));
        }
    }
    Ok(names)
}

fn join_paths(parent: &str, child: &str) -> String {
    let path = [parent, child]
        .into_iter()
        .flat_map(|value| value.trim_matches('/').split('/'))
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    format!("/{path}")
}

fn validate_route(path: &str, span: proc_macro2::Span) -> Result<(), syn::Error> {
    if path.contains(['?', '#']) {
        return Err(syn::Error::new(
            span,
            "RPC routes must not contain a query string or fragment",
        ));
    }
    Ok(())
}

fn normalize_method(method: &str) -> String {
    method.to_ascii_uppercase()
}

fn validate_method(method: &str, span: proc_macro2::Span) -> Result<(), syn::Error> {
    if matches!(
        method,
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    ) {
        Ok(())
    } else {
        Err(syn::Error::new(
            span,
            format!("unsupported HTTP method {method}"),
        ))
    }
}
