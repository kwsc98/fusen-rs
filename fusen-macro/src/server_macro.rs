use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, FnArg, ImplItem, ItemImpl};

use crate::parse_attr;

pub fn fusen_server(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_attr(attr);
    let version = match attr.get("version"){
        Some(version) => quote!(Some(&#version)),
        None => quote!(None),
    };
    let package = match attr.get("package"){
        Some(package) => quote!(#package),
        None => quote!("fusen"),
    };
    let org_item = parse_macro_input!(item as ItemImpl);
    let item = org_item.clone();
    let org_item = get_server_item(org_item);
    let item_trait = &item.trait_.unwrap().1.segments[0].ident;
    let item_self = item.self_ty;
    let items_ident_fn = item.items.iter().fold(vec![], |mut vec, e| {
        if let ImplItem::Fn(fn_item) = e {
            vec.push(fn_item.sig.ident.clone())
        }
        vec
    });
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
                        param.res = Err(fusen::fusen_common::RpcError::Server(err.to_string()));
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
                            Err(err) => Err(fusen::fusen_common::RpcError::Server(err.to_string()))
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

        impl fusen::fusen_common::RpcServer for #item_self {
            fn invoke (&self, param : fusen::fusen_common::FusenMsg) -> fusen::fusen_common::FusenFuture<fusen::fusen_common::FusenMsg> {
                let rpc = self.clone();
                Box::pin(async move {rpc.prv_invoke(param).await})
            }
            fn get_info(&self) -> (&str , &str , Option<&str> , Vec<String>) {
               let mut methods = vec![];
               #(
                  methods.push(stringify!(#items_ident_fn).to_string());
               )*
               (#package ,stringify!(#item_trait) , #version ,methods)
            }
        }

        impl #item_self {
            async fn prv_invoke (&self, mut param : fusen::fusen_common::FusenMsg) -> fusen::fusen_common::FusenMsg {
                #(#items_fn)*
                param.res = Err(fusen::fusen_common::RpcError::Server(format!("not find method by {}",param.method_name)));
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