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
    let methods_info = match get_resource_by_service(org_item.clone()) {
        Ok(methods_info) => methods_info.into_iter().fold(vec![], |mut vec, e| {
            vec.push(serde_json::to_string(&e).unwrap());
            vec
        }),
        Err(err) => return err.into_compile_error().into(),
    };
    let id = match attr.id {
        Some(id) => {
            quote!(#id)
        }
        None => {
            let id = org_item.trait_.as_ref().unwrap().1.segments[0]
                .ident
                .to_string();
            quote!(#id)
        }
    };
    let item = org_item.clone();
    let org_item = get_service_item(org_item);
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
                            let #request : #request_type  = fusen_rs::fusen_internal_common::serde_json::from_value(req_poi_param.remove(idx))
                                   .map_err(|error| fusen_rs::error::FusenError::HttpError(fusen_rs::protocol::fusen::response::HttpStatus { status : 400,message : Some(format!("{error:?}"))}))?;
                            idx += 1;
                    };
                    req_pat.push(request);
                    req_type.push(request_type);
                    vec.push(token);
                }
                vec
            },
            );
            vec.push(quote! {
                if &context.method_info.method_name == stringify!(#method) {
                    let fields = [#(
                        (stringify!(#req_pat),stringify!(#req_type)),
                    )*];
                    let mut req_poi_param = match context.request.get_bodys(&fields) {
                        Ok(result) => result,
                        Err(error) => {
                            return Err(fusen_rs::error::FusenError::Error(Box::new(error)));
                        }
                    };
                    let mut idx = 0;
                    #(
                       #request
                    )*
                    let result = self.#method(
                       #(
                          #req_pat,
                       )*).await;
                    context.response.init_response(result);
                    return Ok(context);
               }
            })
        }
        vec
    });
    let expanded = quote! {

        #org_item

        impl fusen_rs::server::rpc::RpcService for #item_self {
            fn invoke (&'static self, context : fusen_rs::protocol::fusen::context::FusenContext) -> fusen_rs::fusen_internal_common::BoxFuture<Result<fusen_rs::protocol::fusen::context::FusenContext,fusen_rs::error::FusenError>> {
                let rpc = self;
                Box::pin(async move {rpc.prv_invoke(context).await})
            }
            fn get_service_info(&self) -> fusen_rs::protocol::fusen::service::ServiceInfo {
               let service_desc =  fusen_rs::protocol::fusen::service::ServiceDesc::new(#id,#version,#group);
               let mut methods : Vec<fusen_rs::protocol::fusen::service::MethodInfo> = vec![];
               #(
                   let (method_name,method,path,fields) : (String,String,String,Vec<(String,String)>) = fusen_rs::fusen_internal_common::serde_json::from_str(#methods_info).unwrap();
                   methods.push(fusen_rs::protocol::fusen::service::MethodInfo::new(service_desc.clone(),method_name,method,path,fields));
               )*
               fusen_rs::protocol::fusen::service::ServiceInfo::new(service_desc,methods)
            }
        }

        impl #item_self {
            async fn prv_invoke (&self, mut context : fusen_rs::protocol::fusen::context::FusenContext) -> Result<fusen_rs::protocol::fusen::context::FusenContext,fusen_rs::error::FusenError> {
                #(#items_fn)*
                return Err(fusen_rs::error::FusenError::Impossible);
            }
        }
    };
    expanded.into()
}

fn get_service_item(item: ItemImpl) -> proc_macro2::TokenStream {
    let impl_item = item.impl_token;
    let trait_ident = item.trait_.unwrap().1;
    let ident = item.self_ty.to_token_stream();
    let fn_items = item.items;
    quote! {
        #impl_item #trait_ident for #ident {
            #(
                #fn_items
            )*
        }
    }
}

fn get_resource_by_service(
    item: ItemImpl,
) -> Result<Vec<(String, String, String, Vec<(String, String)>)>, syn::Error> {
    let mut method_infos = vec![];
    let attrs = &item.attrs;
    let resource = get_asset_by_attrs(attrs)?;
    let parent_path = match resource.path {
        Some(id) => id,
        None => "/".to_owned() + &item.trait_.unwrap().1.segments[0].ident.to_string(),
    };
    let parent_method = match resource.method {
        Some(method) => method,
        None => "POST".to_string(),
    };
    for fn_item in item.items.iter() {
        if let ImplItem::Fn(item_fn) = fn_item {
            let resource = get_asset_by_attrs(&item_fn.attrs)?;
            let path = match resource.path {
                Some(path) => path,
                None => "/".to_owned() + &item_fn.sig.ident.to_string(),
            };
            let method = match resource.method {
                Some(method) => method,
                None => parent_method.clone(),
            };
            let mut parent_path = parent_path.clone();
            parent_path.push_str(&path);
            let mut fields = vec![];
            for item in &item_fn.sig.inputs {
                if let FnArg::Typed(input) = item {
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
