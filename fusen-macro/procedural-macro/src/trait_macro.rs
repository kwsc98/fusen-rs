use crate::{FusenAttr, fusen_crate_path, get_asset_by_attrs};
use proc_macro::TokenStream;
use quote::{ToTokens, format_ident, quote};
use std::collections::HashSet;
use syn::{FnArg, ItemTrait, ReturnType, TraitItem, parse_macro_input};

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
    let group = attr
        .group
        .map_or_else(|| quote!(None), |value| quote!(Some(#value)));
    let version = attr
        .version
        .map_or_else(|| quote!(None), |value| quote!(Some(#value)));
    let input = parse_macro_input!(item as ItemTrait);
    if input.items.iter().any(
        |item| matches!(item, TraitItem::Fn(method) if method.sig.ident == "__fusen_service_info"),
    ) {
        return syn::Error::new_spanned(
            &input.ident,
            "__fusen_service_info is reserved by fusen_trait",
        )
        .into_compile_error()
        .into();
    }
    let resources = match resources(&input) {
        Ok(resources) => resources,
        Err(error) => return error.into_compile_error().into(),
    };
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
            let asyncness = method.sig.asyncness;
            let ident = &method.sig.ident;
            let inputs = &method.sig.inputs;
            let arguments = inputs.iter().filter_map(|input| match input {
                FnArg::Typed(argument) => Some(&argument.pat),
                FnArg::Receiver(_) => None,
            });
            let output_type = match &method.sig.output {
                ReturnType::Default => quote!(()),
                ReturnType::Type(_, output) => output.to_token_stream(),
            };
            quote! {
                pub #asyncness fn #ident(#inputs) -> Result<#output_type, #runtime::error::FusenError> {
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
    let attrs = &item.attrs;
    let vis = &item.vis;
    let ident = &item.ident;
    let methods = item.items.iter().filter_map(|item| match item {
        TraitItem::Fn(method) => {
            let attrs = &method.attrs;
            let asyncness = &method.sig.asyncness;
            let ident = &method.sig.ident;
            let inputs = &method.sig.inputs;
            let output = match &method.sig.output {
                ReturnType::Default => quote!(()),
                ReturnType::Type(_, output) => output.to_token_stream(),
            };
            Some(quote! {
                #(#attrs)*
                #asyncness fn #ident(#inputs) -> Result<#output, #runtime::error::FusenError>;
            })
        }
        _ => None,
    });
    quote! {
        #(#attrs)*
        #[allow(async_fn_in_trait)]
        #[allow(non_snake_case)]
        #vis trait #ident {
            #(#methods)*

            #[doc(hidden)]
            fn __fusen_service_info() -> #runtime::protocol::fusen::service::ServiceInfo
            where
                Self: Sized,
            {
                #service_info
            }
        }
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

fn resources(item: &ItemTrait) -> Result<Vec<MethodResource>, syn::Error> {
    let parent = get_asset_by_attrs(&item.attrs)?;
    let parent_path = parent.path.unwrap_or_else(|| format!("/{}", item.ident));
    let parent_method = parent.method.unwrap_or_else(|| "POST".into());
    let mut resources = Vec::new();
    for trait_item in &item.items {
        let TraitItem::Fn(method) = trait_item else {
            continue;
        };
        if method.sig.asyncness.is_none() {
            return Err(syn::Error::new_spanned(method, "RPC methods must be async"));
        }
        let resource = get_asset_by_attrs(&method.attrs)?;
        let verb = resource.method.unwrap_or_else(|| parent_method.clone());
        validate_method(&verb, method.sig.ident.span())?;
        let child_path = resource
            .path
            .unwrap_or_else(|| format!("/{}", method.sig.ident));
        let path = join_paths(&parent_path, &child_path);
        let placeholders = placeholders(&path, method.sig.ident.span())?;
        let mut parameter_names = HashSet::new();
        let mut parameters = Vec::new();
        for input in &method.sig.inputs {
            let FnArg::Typed(input) = input else {
                continue;
            };
            let syn::Pat::Ident(pattern) = input.pat.as_ref() else {
                return Err(syn::Error::new_spanned(
                    &input.pat,
                    "RPC parameters must use identifier patterns",
                ));
            };
            let name = pattern
                .ident
                .to_string()
                .trim_start_matches("r#")
                .to_owned();
            parameter_names.insert(name.clone());
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

fn placeholders(path: &str, span: proc_macro2::Span) -> Result<HashSet<String>, syn::Error> {
    let mut names = HashSet::new();
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
