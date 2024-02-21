// lib.rs

use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use quote::__private::ext::RepToTokensExt;
use syn::{self, DeriveInput, ItemTrait, parse_macro_input, ReturnType, TraitItem};

// 定义过程宏来生成struct
#[proc_macro_attribute]
pub fn generateStruct(_: TokenStream, item: TokenStream) -> TokenStream {
    // 解析输入项
    let input = parse_macro_input!(item as ItemTrait);
    let ident = &input.ident;
    let vis = &input.vis;
    let items = &input.items;
    let mut sig_item = vec![];
    for item in items {
        if let TraitItem::Fn(item) = item {
            let ds = &item.sig;
            eprintln!("-----");
            eprintln!("{:?}", ds.asyncness.to_token_stream().to_string());
            eprintln!("{:?}", ds.ident.to_token_stream().to_string());
            eprintln!("{:?}", ds.inputs.to_token_stream().to_string());
            eprintln!("{:?}", ds.output.to_token_stream().to_string());

            eprintln!("-----");

            sig_item.push(ds.clone());
            eprintln!("");
            eprintln!("");
        }
    }
    let mut fn_quote = vec![];
    for item in sig_item {
        let asyncness = item.asyncness;
        let ident = item.ident;
        let inputs = item.inputs;
        let output = item.output;
        fn_quote.push(
            quote! {
                     #[allow(non_snake_case)]
                     #asyncness fn #ident (#inputs) #output {
                    // let mut req_vec : Vec<String> = vec![];
                    // $(
                    //     let mut res_str = serde_json::to_string(&$req);
                    //     if let Err(err) = res_str {
                    //         return Err(krpc_common::RpcError::Client(err.to_string()));
                    //     }
                    //     req_vec.push(res_str.unwrap());
                    // )*
                    // let version : Option<&str> = None;
                    // let msg = krpc_common::KrpcMsg::new(
                    //     krpc_common::get_uuid(),
                    //     version.map(|e|e.to_string()),
                    //     "$package".to_owned() + "." + stringify!(#sig_item.ident),
                    //     stringify!(#sig_item.ident).to_string(),
                    //     req_vec,
                    //     Err(krpc_common::RpcError::Null)
                    // );
                    // let res : Result<#sig_item.output,krpc_common::RpcError> = &self.invoke::<#sig_item.output>(msg).await;
                    // return res;
                }
            }
        );
    }

    // 生成结构体的代码
    let expanded = quote! {
        #vis struct #ident {
            client : &'static krpc_core::client::KrpcClient
        }
        impl #ident {
        #(
                #fn_quote
        )*
       }

    };
    eprintln!("{:?}", expanded.to_string());

    // 返回生成的代码作为TokenStream
    TokenStream::from(expanded)
}
