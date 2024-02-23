use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use std::collections::HashMap;
use syn::{self, parse_macro_input, FnArg, ImplItem, ItemImpl, ItemTrait, ReturnType, TraitItem};

#[proc_macro_attribute]
pub fn rpc_trait(attr: TokenStream, item: TokenStream) -> TokenStream {
    let (package, version) = parse_attr(attr);
    let input = parse_macro_input!(item as ItemTrait);
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
        let req = inputs.iter().fold(vec![], |mut vec, e| {
            if let FnArg::Typed(req) = e {
                vec.push(req.pat.clone());
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
        fn_quote.push(
            quote! {
                    #[allow(non_snake_case)]
                    pub #asyncable fn #ident (#inputs) -> Result<#output_type,krpc_common::RpcError> {
                    let mut req_vec : Vec<String> = vec![];
                    #(
                        let mut res_str = serde_json::to_string(&#req);
                        if let Err(err) = res_str {
                            return Err(krpc_common::RpcError::Client(err.to_string()));
                        }
                        req_vec.push(res_str.unwrap());
                    )*
                    let version : Option<&str> = #version;
                    let msg = krpc_common::KrpcMsg::new(
                        krpc_common::get_uuid(),
                        version.map(|e|e.to_string()),
                        #package.to_owned() + "." + stringify!(#trait_ident),
                        stringify!(#ident).to_string(),
                        req_vec,
                        Err(krpc_common::RpcError::Null)
                    );
                    let res : Result<#output_type,krpc_common::RpcError> = self.client.invoke::<#output_type>(msg).await;
                    return res;
                }
            }
        );
    }
    let rpc_client = syn::Ident::new(&format!("{}Rpc", trait_ident), trait_ident.span());
    let expanded = quote! {
        #item_trait

        #vis struct #rpc_client {
            client : &'static krpc_core::client::KrpcClient
        }
        impl #rpc_client {
        #(
            #fn_quote
        )*
        pub fn new(client : &'static krpc_core::client::KrpcClient) -> #rpc_client {
            #rpc_client {client}
        }
       }

    };
    TokenStream::from(expanded)
}

#[proc_macro_attribute]
pub fn rpc_server(attr: TokenStream, item: TokenStream) -> TokenStream {
    let (package, version) = parse_attr(attr);
    let org_item = parse_macro_input!(item as ItemImpl);
    let item = org_item.clone();
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
                     let result : Result<#req_type,_>  = serde_json::from_slice(req[idx].as_bytes());
                    if let Err(err) = result {
                        param.res = Err(krpc_common::RpcError::Server(err.to_string()));
                        return param;
                    }
                    let #req : #req_type = serde_json::from_slice(req[idx].as_bytes()).unwrap();
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
                let req = &param.req;
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
                            Err(err) => Err(krpc_common::RpcError::Server(err.to_string()))
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

        impl krpc_common::RpcServer for #item_self {
            fn invoke (&self, param : krpc_common::KrpcMsg) -> krpc_common::KrpcFuture<krpc_common::KrpcMsg> {
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
            async fn prv_invoke (&self, mut param : krpc_common::KrpcMsg) -> krpc_common::KrpcMsg {
                #(#items_fn)*
                param.res = Err(krpc_common::RpcError::Server(format!("not find method by {}",param.method_name)));
                return param;
            }
        }
    };
    expanded.into()
}

fn parse_attr(attr: TokenStream) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    let mut map = HashMap::new();
    let attr = attr.clone().to_string();
    let args: Vec<&str> = attr.split(",").collect();
    for arg in args {
        let arg = arg.replace(" ", "");
        let item: Vec<&str> = arg.split("=").collect();
        map.insert(
            item[0].to_string().clone(),
            item[1].replace("\"", "").to_string().clone(),
        );
    }
    let package = map.get("package").map_or("krpc", |e| e);
    let package = quote!(#package);
    let version = match map.get("version").map(|e| e.to_string()) {
        None => quote!(None),
        Some(version) => quote!(Some(&#version)),
    };
    return (package, version);
}

fn get_item_trait(item: ItemTrait) -> proc_macro2::TokenStream {
    let trait_ident = &item.ident;
    let item_fn= item.items.iter().fold(vec![], |mut vec, e| {
        if let TraitItem::Fn(item_fn) = e {
            let asyncable = &item_fn.sig.asyncness;
            let ident = &item_fn.sig.ident;
            let inputs = &item_fn.sig.inputs;
            let output_type = match &item_fn.sig.output {
                ReturnType::Default => {
                    quote! {()}
                }
                ReturnType::Type(_, res_type) => res_type.to_token_stream(),
            };
            vec.push(quote!(
               #asyncable fn #ident (#inputs) -> krpc_common::RpcResult<#output_type>;
            ));
        }
        vec
    },
    );
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
