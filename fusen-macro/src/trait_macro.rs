use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, FnArg, ItemTrait, ReturnType, TraitItem};
use crate::parse_attr;

pub fn fusen_trait(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_attr(attr);
    let version = match attr.get("version"){
        Some(version) => quote!(Some(&#version)),
        None => quote!(None),
    };
    let package = match attr.get("package"){
        Some(package) => quote!(#package),
        None => quote!("fusen"),
    };
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
                    pub #asyncable fn #ident (#inputs) -> Result<#output_type,fusen::fusen_common::RpcError> {
                    let mut req_vec : Vec<String> = vec![];
                    #(
                        let mut res_poi_str = serde_json::to_string(&#req);
                        if let Err(err) = res_poi_str {
                            return Err(fusen::fusen_common::RpcError::Client(err.to_string()));
                        }
                        req_vec.push(res_poi_str.unwrap());
                    )*
                    let version : Option<&str> = #version;
                    let msg = fusen::fusen_common::FusenMsg::new(
                        fusen::fusen_common::get_uuid(),
                        version.map(|e|e.to_string()),
                        #package.to_owned() + "." + stringify!(#trait_ident),
                        stringify!(#ident).to_string(),
                        req_vec,
                        Err(fusen::fusen_common::RpcError::Null)
                    );
                    let res : Result<#output_type,fusen::fusen_common::RpcError> = self.client.invoke::<#output_type>(msg).await;
                    return res;
                }
            }
        );
    }
    let rpc_client = syn::Ident::new(&format!("{}Rpc", trait_ident), trait_ident.span());
    let expanded = quote! {
        #item_trait

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

        pub fn get_info(&self) -> (&str,Vec<&str>) {
            let mut vec : Vec<&str> = vec![];
            
            (stringify!(#trait_ident),vec)
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
            let output_type = match &item_fn.sig.output {
                ReturnType::Default => {
                    quote! {()}
                }
                ReturnType::Type(_, res_type) => res_type.to_token_stream(),
            };
            vec.push(quote!(
               
               #asyncable fn #ident (#inputs) -> fusen::fusen_common::RpcResult<#output_type>;
            ));
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