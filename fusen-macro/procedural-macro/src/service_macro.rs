use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::{FnArg, ImplItem, ItemImpl, parse_macro_input};

use crate::{FusenAttr, get_asset_by_attrs};

pub fn fusen_service(attr: FusenAttr, item: TokenStream) -> TokenStream {
    let version = match attr.version {
        Some(version) => quote!(Some(&#version)),
        None => quote!(None),
    };
    let group = match attr.group {
        Some(group) => quote!(Some(&#group)),
        None => quote!(None),
    };
    let org_item = parse_macro_input!(item as ItemImpl);
    if org_item.trait_.is_none() {
        return syn::Error::new_spanned(&org_item, "fusen_service requires a trait implementation")
            .into_compile_error()
            .into();
    }
    let method_resources = match get_resource_by_service(org_item.clone()) {
        Ok(methods_info) => methods_info,
        Err(err) => return err.into_compile_error().into(),
    };
    let methods_info = method_resources
        .iter()
        .map(method_builder)
        .collect::<Vec<_>>();
    let id = match attr.id {
        Some(id) => {
            quote!(#id)
        }
        None => {
            let Some(trait_path) = org_item.trait_.as_ref().map(|value| &value.1) else {
                return syn::Error::new_spanned(
                    &org_item,
                    "fusen_service requires a trait implementation",
                )
                .into_compile_error()
                .into();
            };
            let Some(segment) = trait_path.segments.last() else {
                return syn::Error::new_spanned(trait_path, "service trait path is empty")
                    .into_compile_error()
                    .into();
            };
            let id = segment.ident.to_string();
            quote!(#id)
        }
    };
    let item = org_item.clone();
    let item_self = item.self_ty;
    let items_fn = item.items.iter().fold(vec![], |mut vec, e| {
        if let ImplItem::Fn(fn_item) = e {
            let method = &fn_item.sig.ident;
            let mut req_pat = vec![];
            let mut req_type = vec![];
            let request = fn_item.sig.inputs.iter().fold(vec![], |mut vec, e| {
                if let FnArg::Typed(input) = e {
                    let request = &input.pat;
                    let request_type = &input.ty;
                    let token = quote! {
                        let #request : #request_type  = fusen_rs::fusen_internal_common::serde_json::from_value(req_poi_paramlkj.pop_front().ok_or_else(|| fusen_rs::error::FusenError::InvalidRequest("request argument count mismatch".into()))?)
                            .map_err(|error| fusen_rs::error::FusenError::InvalidRequest(error.to_string()))?;
                    };
                    req_pat.push(request);
                    req_type.push(request_type);
                    vec.push(token);
                }
                vec
            },
            );
            vec.push(quote! {
                if &context_poiw1.method_info.method_name == stringify!(#method) {
                    let fields = [#(
                        (stringify!(#req_pat),stringify!(#req_type)),
                    )*];
                    let mut req_poi_paramlkj = context_poiw1.request.get_bodys(&fields)?;
                    #(
                       #request
                    )*
                    let result = self.#method(
                       #(
                       #req_pat,
                       )*).await;
                    let mut response = fusen_rs::protocol::fusen::response::FusenResponse::default();
                    response.protocol = context_poiw1.request.protocol;
                    response.init_response(result)?;
                    context_poiw1.response = Some(response);
                    return Ok(context_poiw1);
               }
            })
        }
        vec
    });
    let expanded = quote! {

        #org_item

        impl fusen_rs::filter::FusenFilter for #item_self {
            fn call<'a>(&'a self, join_point : fusen_rs::filter::ProceedingJoinPoint) -> fusen_rs::fusen_internal_common::BoxFutureV2<'a,Result<fusen_rs::protocol::fusen::context::FusenContext,fusen_rs::error::FusenError>> {
                let rpc = self;
                Box::pin(async move {
                    rpc.prv_invoke(join_point.context).await
                })
            }
        }

        impl fusen_rs::server::rpc::RpcService for #item_self {
            fn get_service_info(&self) -> fusen_rs::protocol::fusen::service::ServiceInfo {
               let service_desc =  fusen_rs::protocol::fusen::service::ServiceDesc::new(#id,#version,#group);
               let mut methods : Vec<fusen_rs::protocol::fusen::service::MethodInfo> = vec![];
               #(#methods_info)*
               fusen_rs::protocol::fusen::service::ServiceInfo::new(service_desc,methods)
            }
        }

        impl #item_self {
            async fn prv_invoke (&self, mut context_poiw1 : fusen_rs::protocol::fusen::context::FusenContext) -> Result<fusen_rs::protocol::fusen::context::FusenContext,fusen_rs::error::FusenError> {
                #(#items_fn)*
                return Err(fusen_rs::error::FusenError::RouteNotFound(context_poiw1.method_info.method_name.clone()));
            }
        }
    };
    expanded.into()
}

fn method_builder(
    (name, method, path, fields): &(String, String, String, Vec<(String, String)>),
) -> proc_macro2::TokenStream {
    let fields = fields
        .iter()
        .map(|(name, kind)| quote!((#name.to_owned(), #kind.to_owned())));
    quote! {
        methods.push(fusen_rs::protocol::fusen::service::MethodInfo::new(
            service_desc.clone(), #name.to_owned(), #method.to_owned(), #path.to_owned(), vec![#(#fields),*]
        ));
    }
}

#[allow(clippy::type_complexity)]
fn get_resource_by_service(
    item: ItemImpl,
) -> Result<Vec<(String, String, String, Vec<(String, String)>)>, syn::Error> {
    let mut method_infos = vec![];
    let attrs = &item.attrs;
    let resource = get_asset_by_attrs(attrs)?;
    let trait_name = item
        .trait_
        .as_ref()
        .and_then(|value| value.1.segments.last())
        .ok_or_else(|| {
            syn::Error::new_spanned(&item, "fusen_service requires a trait implementation")
        })?
        .ident
        .to_string();
    let parent_path = match resource.path {
        Some(id) => id,
        None => "/".to_owned() + &trait_name,
    };
    let parent_method = match resource.method {
        Some(method) => method,
        None => "POST".to_string(),
    };
    for fn_item in item.items.iter() {
        if let ImplItem::Fn(item_fn) = fn_item {
            if item_fn.sig.asyncness.is_none() {
                return Err(syn::Error::new_spanned(
                    item_fn,
                    "RPC methods must be async",
                ));
            }
            let resource = get_asset_by_attrs(&item_fn.attrs)?;
            let path = match resource.path {
                Some(path) => path,
                None => "/".to_owned() + &item_fn.sig.ident.to_string(),
            };
            let method = match resource.method {
                Some(method) => method,
                None => parent_method.clone(),
            };
            validate_method(&method, item_fn.sig.ident.span())?;
            let mut parent_path = parent_path.clone();
            parent_path.push_str(&path);
            let mut fields = vec![];
            for item in &item_fn.sig.inputs {
                if let FnArg::Typed(input) = item {
                    if !matches!(input.pat.as_ref(), syn::Pat::Ident(_)) {
                        return Err(syn::Error::new_spanned(
                            &input.pat,
                            "RPC parameters must use identifier patterns",
                        ));
                    }
                    let request = &input.pat;
                    let request_type = &input.ty;
                    fields.push((
                        request.into_token_stream().to_string(),
                        request_type.into_token_stream().to_string(),
                    ));
                }
            }
            method_infos.push((item_fn.sig.ident.to_string(), method, parent_path, fields));
        }
    }
    Ok(method_infos)
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
