use fusen_derive_macro::fusen_attr;
use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse::Parser, parse_macro_input, Attribute, DeriveInput, Meta};

mod data;
mod handler_macro;
mod server_macro;
mod trait_macro;

#[proc_macro_attribute]
pub fn handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = HandlerAttr::from_attr(attr);
    match attr {
        Ok(attr) => handler_macro::fusen_handler(attr, item),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn fusen_trait(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = FusenAttr::from_attr(attr);
    match attr {
        Ok(attr) => trait_macro::fusen_trait(attr, item),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro_derive(Data)]
pub fn data(item: TokenStream) -> TokenStream {
    data::data(item)
}

#[proc_macro_attribute]
pub fn fusen_server(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = FusenAttr::from_attr(attr);
    match attr {
        Ok(attr) => server_macro::fusen_server(attr, item),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn asset(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn url_config(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = UrlConfigAttr::from_attr(attr);
    if let Err(err) = attr {
        return err.into_compile_error().into();
    }
    let attr = attr.unwrap();
    let attr = attr.attr;
    let org_item = parse_macro_input!(item as DeriveInput);
    let Some(attr) = attr else {
        return syn::Error::new_spanned(
            org_item.to_token_stream(),
            "url_config must label to attr",
        )
        .into_compile_error()
        .into();
    };
    let id = &org_item.ident;
    let token = quote! {

        #[derive(serde::Serialize, serde::Deserialize, Default, fusen_procedural_macro::Data)]
        #org_item

        impl #id {
            pub fn from_url(url : &str) -> Result<Self,fusen_common::Error> {
                let info : Vec<&str> = url.split("://").collect();
                if info[0] != #attr {
                   return Err(format!("err1 url {}",url).into());
                }
                let info : Vec<&str> = info[1].split("?").collect();
                if info[0] != stringify!(#id) {
                    return Err(format!("err2 url {}",url).into());
                }
                fusen_common::url::from_url(info[1])
            }
            pub fn to_url(&self) -> Result<String, fusen_common::Error> {
                let mut res = String::new();
                res.push_str(&(#attr.to_owned() + "://" + stringify!(#id) + "?" ));
                res.push_str(&(fusen_common::url::to_url(self)?));
                Ok(res)
            }
        }
    };
    token.into()
}

fn get_asset_by_attrs(attrs: &Vec<Attribute>) -> Result<ResourceAttr, syn::Error> {
    for attr in attrs {
        if let Meta::List(list) = &attr.meta {
            if let Some(segment) = list.path.segments.first() {
                if segment.ident == "asset" {
                    return ResourceAttr::from_attr(list.tokens.clone().into());
                }
            }
        }
    }
    Ok(ResourceAttr::default())
}

fusen_attr! {
    ResourceAttr,
    path,
    method
}

fusen_attr! {
    UrlConfigAttr,
    attr
}

fusen_attr! {
    FusenAttr,
    id,
    version,
    group
}

fusen_attr! {
    HandlerAttr,
    id
}
