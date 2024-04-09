use std::collections::HashMap;

use crate::{get_asset_by_attrs, FusenAttr};
use fusen_common::MethodResource;
use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, FnArg, ItemTrait, ReturnType, TraitItem};

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
    let (id, spring_cloud_name, methods_info) = match get_resource_by_trait(input.clone()) {
        Ok(methods_info) => {
            let methods = methods_info.2.into_iter().fold(vec![], |mut vec, e| {
                vec.push(e.to_json_str());
                let MethodResource {
                    id,
                    path,
                    name,
                    method,
                } = e;
                methods_cache.insert(name, (id, path, method));
                vec
            });
            (methods_info.0, methods_info.1, methods)
        }
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
        let req = inputs.iter().fold(vec![], |mut vec, e| {
            if let FnArg::Typed(req) = e {
                vec.push(req.pat.clone());
                fields.push(req.pat.to_token_stream().to_string());
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
        let (methos_id, methos_path, methos_type) = methods_cache.get(&ident.to_string()).unwrap();
        fn_quote.push(
            quote! {
                    #[allow(non_snake_case)]
                    pub #asyncable fn #ident (#inputs) -> Result<#output_type,fusen::fusen_common::error::FusenError> {
                    let mut req_vec : Vec<String> = vec![];
                    let mut fields : Vec<String> = vec![];
                    #(
                        fields.push(#fields.to_string());
                    )*
                    #(
                        let mut res_poi_str = serde_json::to_string(&#req);
                        if let Err(err) = res_poi_str {
                            return Err(fusen::fusen_common::error::FusenError::Client(err.to_string()));
                        }
                        req_vec.push(res_poi_str.unwrap());
                    )*
                    let version : Option<&str> = #version;
                    let group : Option<&str> = #group;
                    let mut mate_data = fusen::fusen_common::MetaData::new();
                    mate_data.insert("spring_cloud_name".to_string(),#spring_cloud_name.to_string());
                    let msg = fusen::fusen_common::FusenContext::new (
                        fusen::fusen_common::logs::get_uuid(),
                        fusen::fusen_common::Path::new(#methos_type,#methos_path.to_string()),
                        mate_data,
                        version.map(|e|e.to_string()),
                        group.map(|e|e.to_string()),
                        #package.to_owned(),
                        #methos_id.to_string(),
                        req_vec,
                        fields
                    );
                    let res : Result<#output_type,fusen::fusen_common::error::FusenError> = self.client.invoke::<#output_type>(msg).await;
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
            client : &'static fusen::client::FusenClient
        }
        impl #rpc_client {
        #(
            #fn_quote
        )*
        pub fn new(client : &'static fusen::client::FusenClient) -> #rpc_client {
            #rpc_client {client}
        }

        pub fn get_info(&self) -> fusen::fusen_common::server::ServerInfo {
            let mut methods : Vec<fusen::fusen_common::MethodResource> = vec![];
            #(
                methods.push(fusen::fusen_common::MethodResource::form_json_str(#methods_info));
            )*
            fusen::fusen_common::server::ServerInfo::new(#package,#version,#group,methods)
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
                   #asyncable fn #ident (#inputs) -> fusen::fusen_common::FusenResult<#output_type>;
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
) -> Result<(String, String, Vec<MethodResource>), syn::Error> {
    let mut res = vec![];
    let attrs = &item.attrs;
    let resource = get_asset_by_attrs(attrs)?;
    let parent_id = match resource.id {
        Some(id) => id,
        None => item.ident.to_string(),
    };
    let spring_cloud_name = match resource.spring_cloud {
        Some(name) => name,
        None => parent_id.clone(),
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
        if let TraitItem::Fn(item_fn) = fn_item {
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
    return Ok((parent_id, spring_cloud_name, res));
}
