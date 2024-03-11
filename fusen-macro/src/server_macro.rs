use fusen_common::MethodResource;
use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, FnArg, ImplItem, ItemImpl};

use crate::{get_resource_by_attrs, FusenAttr};

pub fn fusen_server(
    attr:FusenAttr,
    item: TokenStream,
) -> TokenStream {
    let version = match attr.version {
        Some(version) => quote!(Some(&#version)),
        None => quote!(None),
    };
    let package = match attr.package {
        Some(package) => quote!(#package),
        None => quote!("fusen"),
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
    let item = org_item.clone();
    let org_item = get_server_item(org_item);
    let item_trait = &item.trait_.unwrap().1.segments[0].ident;
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
                        param.res = Err(fusen::fusen_common::FusenError::Server(err.to_string()));
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
                            Err(err) => Err(fusen::fusen_common::FusenError::Server(err.to_string()))
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
    let temp_method = syn::Ident::new(
        &format!("{}MethodResourceServer", item_trait),
        item_trait.span(),
    );
    let expanded = quote! {
        use fusen::fusen_common::MethodResource as #temp_method;

        #org_item

        impl fusen::fusen_common::RpcServer for #item_self {
            fn invoke (&self, param : fusen::fusen_common::FusenMsg) -> fusen::fusen_common::FusenFuture<fusen::fusen_common::FusenMsg> {
                let rpc = self.clone();
                Box::pin(async move {rpc.prv_invoke(param).await})
            }
            fn get_info(&self) -> (&str , &str , Option<&str> , Vec<#temp_method>) {

               let mut methods : Vec<#temp_method> = vec![];
               #(
                methods.push(#temp_method::form_json_str(#methods_info));
               )*
               (#package ,&#id , #version ,methods)
            }
        }

        impl #item_self {
            async fn prv_invoke (&self, mut param : fusen::fusen_common::FusenMsg) -> fusen::fusen_common::FusenMsg {
                #(#items_fn)*
                param.res = Err(fusen::fusen_common::FusenError::Server(format!("not find method by {}",param.method_name)));
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
    let resource = get_resource_by_attrs(attrs)?;
    let parent_id = match resource.id {
        Some(id) => id,
        None => item.trait_.unwrap().1.segments[0].ident.to_string(),
    };
    let parent_path = match resource.path {
        Some(path) => path,
        None => "/".to_owned() + &parent_id,
    };
    let parent_method = match resource.method {
        Some(method) => method,
        None => "POST".to_string(),
    };

    for fn_item in item.items.iter() {
        if let ImplItem::Fn(item_fn) = fn_item {
            let resource = get_resource_by_attrs(&item_fn.attrs)?;
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
            res.push(MethodResource::new(id, parent_path, method));
        }
    }
    return Ok((parent_id, res));
}