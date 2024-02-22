
use proc_macro::TokenStream;
use std::collections::HashMap;
use quote::{quote, ToTokens};
use syn::{self, FnArg, ItemTrait, parse_macro_input, ReturnType, TraitItem};


#[proc_macro_attribute]
pub fn rpc_resources(attr: TokenStream, item: TokenStream) -> TokenStream {
    let map = parse_attr(attr);
    let package = map.get("package").map_or("krpc",|e|e);
    let package = quote!(#package);
    let version = match map.get("version").map(|e| e.to_string()) {
        None => quote!(None),
        Some(version) => quote!(Some(&#version))
    };
    let input = parse_macro_input!(item as ItemTrait);
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
            ReturnType::Default => { quote! {()} }
            ReturnType::Type(_, res_type) => { res_type.to_token_stream() }
        };
        fn_quote.push(
            quote! {
                     #[allow(non_snake_case)]
                     #asyncable fn #ident (#inputs) -> Result<#output_type,krpc_common::RpcError> {
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


    // 生成结构体的代码
    let expanded = quote! {
        #vis struct #trait_ident {
            client : &'static krpc_core::client::KrpcClient
        }
        impl #trait_ident {
        #(
                #fn_quote
        )*
       }

    };
    TokenStream::from(expanded)
}


fn parse_attr(attr: TokenStream) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let attr = attr.clone().to_string();
    let args: Vec<&str> = attr.split(",").collect();
    for arg in args {
        let arg = arg.replace(" ", "");
        let item: Vec<&str> = arg.split("=").collect();
        map.insert(item[0].to_string().clone(), item[1].replace("\"", "").to_string().clone());
    }
    return map;
}