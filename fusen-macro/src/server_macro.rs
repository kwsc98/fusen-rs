use fusen_common::MethodResource;
use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, FnArg, ImplItem, ItemImpl};

use crate::{get_asset_by_attrs, FusenAttr};

pub fn fusen_server(attr: FusenAttr, item: TokenStream) -> TokenStream {
    let version = match attr.version {
        Some(version) => quote!(Some(&#version)),
        None => quote!(None),
    };
    let group = match attr.group {
        Some(group) => quote!(Some(&#group)),
        None => quote!(None),
    };
    let org_item = parse_macro_input!(item as ItemImpl);
    let (id, methods_info) = match get_resource_by_server(org_item.clone()) {
        Ok(methods_info) => (
            methods_info.0,
            methods_info.1.iter().fold(vec![], |mut vec, e| {
                vec.push(e.to_json_str());
                vec
            }),
        ),
        Err(err) => return err.into_compile_error().into(),
    };
    let package = match attr.package {
        Some(mut package) => {
            package.push('.');
            package.push_str(&id);
            quote!(#package)
        }
        None => quote!(#id),
    };
    let item = org_item.clone();
    let org_item = get_server_item(org_item);
    let item_self = item.self_ty;
    let items_fn = item.items.iter().fold(vec![], |mut vec, e| {
        if let ImplItem::Fn(fn_item) = e {
            let method = &fn_item.sig.ident;
            let mut req_pat = vec![];
            let req = fn_item.sig.inputs.iter().fold(vec![], |mut vec, e| {
                if let FnArg::Typed(input) = e {
                    let req = &input.pat;
                    let req_type = &input.ty;
                    let token = quote! {
                     let result : Result<#req_type,_>  = serde_json::from_slice(req_poi_param[idx].as_bytes());
                    if let Err(err) = result {
                        param.res = Err(fusen_rs::fusen_common::error::FusenError::from(err.to_string()));
                        return param;
                    }
                    let #req : #req_type = result.unwrap();
                    idx += 1;
                    };
                    req_pat.push(req);
                    vec.push(token);
                }
                vec
            },
            );
            vec.push(quote! {
                if &param.method_name[..] == stringify!(#method) {
                let req_poi_param = &param.req;
                let mut idx = 0;
                #(
                    #req
                )*
                let res = self.#method(
                    #(
                        #req_pat,
                    )*
                ).await;
                param.res = match res {
                    Ok(res) => {
                        let res = serde_json::to_string(&res);
                        match res {
                            Ok(res) => Ok(res),
                            Err(err) => Err(fusen_rs::fusen_common::error::FusenError::from(err.to_string()))
                        }
                    },
                    Err(info) => Err(info)
                };
                return param;
            }
            }
            )
        }
        vec
    });
    let expanded = quote! {

        #org_item

        impl fusen_rs::fusen_common::server::RpcServer for #item_self {
            fn invoke (&'static self, param : fusen_rs::fusen_common::FusenContext) -> fusen_rs::fusen_common::FusenFuture<fusen_rs::fusen_common::FusenContext> {
                let rpc = self;
                Box::pin(async move {rpc.prv_invoke(param).await})
            }
            fn get_info(&self) -> fusen_rs::fusen_common::server::ServerInfo {

               let mut methods : Vec<fusen_rs::fusen_common::MethodResource> = vec![];
               #(
                methods.push(fusen_rs::fusen_common::MethodResource::form_json_str(#methods_info));
               )*
               fusen_rs::fusen_common::server::ServerInfo::new(#package,#version,#group,methods)
            }
        }

        impl #item_self {
            async fn prv_invoke (&self, mut param : fusen_rs::fusen_common::FusenContext) -> fusen_rs::fusen_common::FusenContext {
                #(#items_fn)*
                param.res = Err(fusen_rs::fusen_common::error::FusenError::from(format!("not find method by {}",param.method_name)));
                return param;
            }
        }
    };
    expanded.into()
}

fn get_server_item(item: ItemImpl) -> proc_macro2::TokenStream {
    let impl_item = item.impl_token;
    let trait_ident = item.trait_.unwrap().1;
    let ident = item.self_ty.to_token_stream();
    let fn_items = item.items.iter().fold(vec![], |mut vec, e| {
        if let ImplItem::Fn(fn_item) = e {
            vec.push(fn_item);
        }
        vec
    });
    quote! {
        #impl_item #trait_ident for #ident {
            #(
                #[allow(non_snake_case)]
                #fn_items
            )*
        }
    }
}

fn get_resource_by_server(item: ItemImpl) -> Result<(String, Vec<MethodResource>), syn::Error> {
    let mut res = vec![];
    let attrs = &item.attrs;
    let resource = get_asset_by_attrs(attrs)?;
    let parent_id = match resource.id {
        Some(id) => id,
        None => item.trait_.unwrap().1.segments[0].ident.to_string(),
    };
    let parent_path = match resource.path {
        Some(path) => path,
        None => "".to_owned(),
    };
    let parent_method = match resource.method {
        Some(method) => method,
        None => "POST".to_string(),
    };

    for fn_item in item.items.iter() {
        if let ImplItem::Fn(item_fn) = fn_item {
            let resource = get_asset_by_attrs(&item_fn.attrs)?;
            let id = match resource.id {
                Some(id) => id,
                None => item_fn.sig.ident.to_string(),
            };
            let path = match resource.path {
                Some(path) => path,
                None => "/".to_owned() + &id.clone(),
            };
            let method = match resource.method {
                Some(method) => method,
                None => parent_method.clone(),
            };
            let mut parent_path = parent_path.clone();
            parent_path.push_str(&path);
            res.push(MethodResource::new(
                id,
                item_fn.sig.ident.to_string(),
                parent_path,
                method,
            ));
        }
    }
    return Ok((parent_id, res));
}
