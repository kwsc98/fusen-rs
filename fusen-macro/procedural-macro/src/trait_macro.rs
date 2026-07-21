use crate::{FusenAttr, fusen_crate_path, get_asset_by_attrs, is_asset_attr};
use proc_macro::TokenStream;
use quote::{ToTokens, format_ident, quote};
use std::collections::BTreeSet;
use syn::{
    FnArg, GenericParam, Generics, ItemTrait, Pat, ReturnType, Signature, TraitItem, Type,
    TypePath, WherePredicate, parse_macro_input, parse_quote, visit::Visit,
};

#[derive(Clone, Copy)]
enum ParameterSource {
    Path,
    Query,
    Body,
}

struct MethodResource {
    name: String,
    method: String,
    path: String,
    parameters: Vec<(String, ParameterSource)>,
}

pub fn fusen_trait(attr: FusenAttr, item: TokenStream) -> TokenStream {
    let runtime = fusen_crate_path();
    let input = parse_macro_input!(item as ItemTrait);
    let resources = match validate_trait(&input) {
        Ok(resources) => resources,
        Err(error) => return error.into_compile_error().into(),
    };
    let group = attr
        .group
        .map_or_else(|| quote!(None), |value| quote!(Some(#value)));
    let version = attr
        .version
        .map_or_else(|| quote!(None), |value| quote!(Some(#value)));
    let id = attr.id.unwrap_or_else(|| input.ident.to_string());
    let trait_ident = &input.ident;
    let vis = &input.vis;
    let client_ident = format_ident!("{}Client", trait_ident);
    let service_info = service_info_tokens(&runtime, &id, &version, &group, &resources);
    let item_trait = generated_trait(&runtime, &input, &service_info);

    let client_methods = input
        .items
        .iter()
        .filter_map(|item| match item {
            TraitItem::Fn(method) => Some(method),
            _ => None,
        })
        .map(|method| {
            let attrs = method.attrs.iter().filter(|attr| !is_asset_attr(attr));
            let ident = &method.sig.ident;
            let parameters = method
                .sig
                .inputs
                .iter()
                .filter_map(|input| match input {
                    FnArg::Typed(parameter) => Some(parameter),
                    FnArg::Receiver(_) => None,
                })
                .collect::<Vec<_>>();
            let arguments = parameters.iter().map(|parameter| &parameter.pat);
            let output_type = output_type(&method.sig.output);
            quote! {
                #(#attrs)*
                pub async fn #ident(&self, #(#parameters),*) -> Result<#output_type, #runtime::error::FusenError> {
                    let mut arguments = Vec::new();
                    #(
                        arguments.push(
                            #runtime::fusen_internal_common::serde_json::to_value(&#arguments)
                                .map_err(|error| #runtime::error::FusenError::internal(
                                    "failed to serialize request argument",
                                    error,
                                ))?,
                        );
                    )*
                    let response = self.client.invoke(stringify!(#ident), arguments).await?;
                    #runtime::fusen_internal_common::serde_json::from_value(response)
                        .map_err(|error| #runtime::error::FusenError::InvalidResponse(error.to_string()))
                }
            }
        })
        .collect::<Vec<_>>();

    quote! {
        #item_trait

        #[derive(Clone)]
        #vis struct #client_ident {
            client: std::sync::Arc<#runtime::client::FusenClient>,
        }

        #[allow(non_snake_case)]
        impl #client_ident {
            #(#client_methods)*

            pub async fn init(
                context: &mut #runtime::client::FusenClientContext,
                options: #runtime::client::ClientOptions,
            ) -> Result<Self, #runtime::error::FusenError> {
                let client = context.init_client(Self::get_service_info(), options).await?;
                Ok(Self { client: std::sync::Arc::new(client) })
            }

            pub async fn close(&self) -> Result<(), #runtime::error::FusenError> {
                self.client.close().await
            }

            pub fn get_service_info() -> #runtime::protocol::fusen::service::ServiceInfo {
                #service_info
            }
        }
    }
    .into()
}

fn generated_trait(
    runtime: &proc_macro2::TokenStream,
    item: &ItemTrait,
    service_info: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let mut generated = item.clone();
    generated.attrs.retain(|attr| !is_asset_attr(attr));
    for item in &mut generated.items {
        if let TraitItem::Fn(method) = item {
            method.attrs.retain(|attr| !is_asset_attr(attr));
            let output = output_type(&method.sig.output);
            method.sig.output = parse_quote!(
                -> std::result::Result<#output, #runtime::error::FusenError>
            );
        }
    }
    let metadata: TraitItem = parse_quote! {
        #[doc(hidden)]
        fn __fusen_service_info() -> #runtime::protocol::fusen::service::ServiceInfo
        where
            Self: Sized,
        {
            #service_info
        }
    };
    generated.items.push(metadata);
    quote! {
        #[allow(async_fn_in_trait)]
        #[allow(non_snake_case)]
        #generated
    }
}

fn service_info_tokens(
    runtime: &proc_macro2::TokenStream,
    id: &str,
    version: &proc_macro2::TokenStream,
    group: &proc_macro2::TokenStream,
    resources: &[MethodResource],
) -> proc_macro2::TokenStream {
    let methods = resources.iter().map(|resource| {
        let name = &resource.name;
        let path = &resource.path;
        let method = format_ident!("{}", resource.method);
        let parameters = resource.parameters.iter().map(|(name, source)| {
            let source = match source {
                ParameterSource::Path => quote!(Path),
                ParameterSource::Query => quote!(Query),
                ParameterSource::Body => quote!(Body),
            };
            quote! {
                #runtime::protocol::fusen::service::ParameterInfo::new(
                    #name,
                    #runtime::protocol::fusen::service::ParameterSource::#source,
                )
            }
        });
        quote! {
            #runtime::protocol::fusen::service::MethodInfo::new(
                service_desc.clone(),
                #name.to_owned(),
                #runtime::http::Method::#method,
                #path.to_owned(),
                vec![#(#parameters),*],
            )
        }
    });
    quote! {{
        let service_desc = #runtime::protocol::fusen::service::ServiceDesc::new(
            #id,
            #version,
            #group,
        );
        #runtime::protocol::fusen::service::ServiceInfo::new(
            service_desc.clone(),
            vec![#(#methods),*],
        )
    }}
}

fn validate_trait(item: &ItemTrait) -> Result<Vec<MethodResource>, syn::Error> {
    if item.unsafety.is_some() || item.auto_token.is_some() {
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
    if item.items.iter().any(
        |item| matches!(item, TraitItem::Fn(method) if method.sig.ident == "__fusen_service_info"),
    ) {
        return Err(syn::Error::new_spanned(
            &item.ident,
            "__fusen_service_info is reserved by fusen_trait",
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
        || signature.unsafety.is_some()
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
    let explicit_receiver_lifetime = receiver
        .reference
        .as_ref()
        .and_then(|(_, lifetime)| lifetime.as_ref())
        .is_some();
    if receiver.reference.is_none()
        || receiver.mutability.is_some()
        || receiver.colon_token.is_some()
        || explicit_receiver_lifetime
    {
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

fn output_type(output: &ReturnType) -> Type {
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
