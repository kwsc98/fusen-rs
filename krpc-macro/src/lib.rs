use proc_macro::TokenStream;
use std::collections::HashMap;
use proc_macro::Ident;
use quote::{quote, ToTokens};
use syn::{self, parse_macro_input, FnArg, ItemTrait, ReturnType, TraitItem, ItemFn, Item, ItemImpl};

#[proc_macro_attribute]
pub fn rpc_trait(attr: TokenStream, item: TokenStream) -> TokenStream {
    let (package, version) = parse_attr(attr);
    let input = parse_macro_input!(item as ItemTrait);
    let item_trait = input.clone();
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
    let rpc_client = syn::Ident::new(&format!("{}Impl", trait_ident), trait_ident.span());
    let expanded = quote! {
        #item_trait

        #vis struct #rpc_client {
            pub client : &'static krpc_core::client::KrpcClient
        }
        impl #rpc_client {
        #(
            #fn_quote
        )*
       }

    };
    TokenStream::from(expanded)
}


#[proc_macro_attribute]
pub fn rpc_server(attr: TokenStream, item: TokenStream) -> TokenStream {
    let (package, version) = parse_attr(attr);
    let item = parse_macro_input!(item as ItemImpl);
    let item_trait = item.trait_.unwrap();
    let item_self = item.self_ty;
    let items_fn = item.items;
    eprintln!("{:?}", item);
    let expanded = quote! {
        #item

        impl krpc_common::RpcServer for #item_self {
            fn invoke (&self, param : krpc_common::KrpcMsg) -> krpc_common::KrpcFuture<krpc_common::KrpcMsg> {
                let rpc = self.clone();
                Box::pin(async move {

                    rpc.prv_invoke(param).await

                })
            }
            fn get_info(&self) -> (&str , &str , Option<&str> , Vec<String>) {
               let mut methods = vec![];
               #(
                  methods.push(stringify!($method).to_string());
               )*
               (#package ,stringify!(#item_self) , #version ,methods)
            }
        }
    };
    // eprintln!("{:?}", item);
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
