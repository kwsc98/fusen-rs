use crate::{FusenAttr, get_asset_by_attrs};
use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use std::collections::HashMap;
use syn::{FnArg, ItemTrait, ReturnType, TraitItem, parse_macro_input};

pub fn fusen_trait(attr: FusenAttr, item: TokenStream) -> TokenStream {
    let group = match attr.group {
        Some(group) => quote!(Some(&#group)),
        None => quote!(None),
    };
    let version = match attr.version {
        Some(version) => quote!(Some(&#version)),
        None => quote!(None),
    };
    let input = parse_macro_input!(item as ItemTrait);
    let mut methods_cache = HashMap::new();
    let methods_info = match get_resource_by_trait(input.clone()) {
        Ok(methods_info) => methods_info.into_iter().fold(vec![], |mut vec, e| {
            vec.push(serde_json::to_string(&e).unwrap());
            methods_cache.insert(
                e.0.to_owned(),
                (e.1.to_owned(), e.2.to_owned(), e.3.to_owned()),
            );
            vec
        }),
        Err(error) => return error.into_compile_error().into(),
    };
    let id = match attr.id {
        Some(trait_id) => {
            quote!(#trait_id)
        }
        None => {
            let id = input.ident.to_string();
            quote!(#id)
        }
    };
    let item_trait = get_item_trait(input.clone());
    let trait_ident = &input.ident;
    let vis = &input.vis;
    let items = &input.items;
    let mut sig_item = vec![];
    for item in items {
        if let TraitItem::Fn(item) = item {
            sig_item.push(item.sig.clone());
        }
    }
    let mut fn_quote = vec![];
    for item in sig_item {
        let asyncable = item.asyncness;
        let ident = item.ident;
        let inputs = item.inputs;
        let mut fields = vec![];
        let request_pat = inputs.iter().fold(vec![], |mut vec, e| {
            if let FnArg::Typed(request) = e {
                vec.push(request.pat.clone());
                fields.push(request.pat.to_token_stream().to_string());
            }
            vec
        });
        let output = item.output;
        let output_type = match &output {
            ReturnType::Default => {
                quote! {()}
            }
            ReturnType::Type(_, res_type) => res_type.to_token_stream(),
        };
        let (methos_path, methos_type, _) = methods_cache.get(&ident.to_string()).unwrap();
        fn_quote.push(
            quote! {
                pub #asyncable fn #ident (#inputs) -> Result<#output_type,fusen_rs::error::FusenError> {
                    let mut request_body : Vec<String> = vec![];
                    let fields_pat = [
                    #(
                        #fields,
                    )*];
                    #(
                        let mut res_poi_str = serde_json::to_value(&#request_pat);
                        if let Err(error) = res_poi_str {
                            return Err(fusen_rs::fusen_common::error::FusenError::Error(Box::new(error)));
                        }
                        request_body.push(res_poi_str.unwrap());
                    )*
                    let version : Option<&str> = #version;
                    let group : Option<&str> = #group;
                    let mut request = fusen_rs::fusen_common::FusenRequest::new_for_client(#methos_type,fields_ty,request_body);
                    let mut context = fusen_rs::fusen_common::FusenContext::new(
                        fusen_rs::fusen_common::logs::get_uuid(),
                        fusen_rs::fusen_common::ContextInfo::default()
                            .path(fusen_rs::fusen_common::Path::new(#methos_type,#methos_path.to_string()))
                            .version(version.map(|e|e.to_string()))
                            .class_name(#id.to_owned())
                            .method_name(stringify!(#ident).to_string())
                            .group(group.map(|e|e.to_string())),
                        request,
                        mate_data,
                    );
                    context.get_mut_response().insert_return_ty(stringify!(#output_type));
                    let res : Result<#output_type,fusen_rs::fusen_common::error::FusenError> = self.client.invoke::<#output_type>(context).await;
                    return res;
                }
            }
        );
    }
    let rpc_client = syn::Ident::new(&format!("{}Client", trait_ident), trait_ident.span());

    let expanded = quote! {
        #item_trait

        #[derive(Clone)]
        #vis struct #rpc_client {
            service_info: std::sync::Arc<fusen_rs::protocol::fusen::service::ServiceInfo>,
            client : std::sync::Arc<fusen_rs::client::FusenClient>
        }
        impl #rpc_client {
        #(
            #fn_quote
        )*

        pub fn new(client : std::sync::Arc<fusen_rs::client::FusenClient>) -> #rpc_client {
            #rpc_client {
                service_info : std::sync::Arc::new(Self::get_service_info()),
                client
            }
        }

        fn get_service_info() -> fusen_rs::protocol::fusen::service::ServiceInfo {
            let service_desc =  fusen_rs::protocol::fusen::service::ServiceDesc::new(#id,#version,#group);
            let mut methods : Vec<fusen_rs::protocol::fusen::service::MethodInfo> = vec![];
            #(
                let (method_name,method,path,fields) : (String,String,String,Vec<(String,String)>) = fusen_rs::fusen_internal_common::serde_json::from_str(#methods_info).unwrap();
                methods.push(fusen_rs::protocol::fusen::service::MethodInfo::new(service_desc.clone(),method_name,method,path,fields));
            )*
            fusen_rs::protocol::fusen::service::ServiceInfo::new(service_desc,methods)
         }
       }

    };
    TokenStream::from(expanded)
}

fn get_item_trait(item: ItemTrait) -> proc_macro2::TokenStream {
    let trait_ident = &item.ident;
    let item_fn = item.items.iter().fold(vec![], |mut vec, e| {
        if let TraitItem::Fn(item_fn) = e {
            let asyncable = &item_fn.sig.asyncness;
            let ident = &item_fn.sig.ident;
            let inputs = &item_fn.sig.inputs;
            let attrs = &item_fn.attrs;
            let output_type = match &item_fn.sig.output {
                ReturnType::Default => {
                    quote! {()}
                }
                ReturnType::Type(_, res_type) => res_type.to_token_stream(),
            };
            vec.push(quote! {
                   #(#attrs)*
                   #asyncable fn #ident (#inputs) -> fusen_rs::fusen_common::FusenResult<#output_type>;
            });
        }
        vec
    });
    quote! {
        pub trait #trait_ident {
           #(
               #[allow(async_fn_in_trait)]
               #[allow(non_snake_case)]
               #item_fn
            )*
        }
    }
}

fn get_resource_by_trait(
    item: ItemTrait,
) -> Result<Vec<(String, String, String, Vec<(String, String)>)>, syn::Error> {
    let mut method_infos = vec![];
    let attrs = &item.attrs;
    let resource = get_asset_by_attrs(attrs)?;
    let parent_path = match resource.path {
        Some(id) => id,
        None => "/".to_owned() + &item.ident.to_string(),
    };
    let parent_method = match resource.method {
        Some(method) => method,
        None => "POST".to_string(),
    };
    for fn_item in item.items.iter() {
        if let TraitItem::Fn(item_fn) = fn_item {
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
