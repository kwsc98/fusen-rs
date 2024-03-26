use fusen_common::fusen_attr;
use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse::Parser, parse_macro_input, Attribute, DeriveInput, Meta};

mod server_macro;
mod trait_macro;

#[proc_macro_attribute]
pub fn fusen_trait(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = FusenAttr::from_attr(attr);
    match attr {
        Ok(attr) => trait_macro::fusen_trait(attr, item),
        Err(err) => err.into_compile_error().into(),
    }
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
pub fn resource(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_derive(UrlConfig)]
pub fn url_config(org: TokenStream) -> TokenStream {
    let org_item = parse_macro_input!(org as DeriveInput);
    let id = &org_item.ident;
    let token = quote! {
        impl fusen_common::url::UrlConfig for #id {
            fn from_url(url : &str) -> Result<Self,fusen_common::Error> {
                let info : Vec<&str> = url.split("://").collect();
                if info[0] != "config" {
                   return Err(format!("err url {}",url).into());
                }
                let info : Vec<&str> = info[1].split("?").collect();
                if info[0] != stringify!(#id) {
                    return Err(format!("err url {}",url).into());
                }
                fusen_common::url::from_url(info[1])
            }
            fn to_url(&self) -> Result<String, fusen_common::Error> {
                let mut res = String::new();
                res.push_str(&("config://".to_owned() + stringify!(#id) + "?" ));
                res.push_str(&(fusen_common::url::to_url(self)?));
                Ok(res)
            }
        }
    };
    token.into()
}

fn get_resource_by_attrs(attrs: &Vec<Attribute>) -> Result<ResourceAttr, syn::Error> {
    for attr in attrs {
        if let Meta::List(list) = &attr.meta {
            if let Some(segment) = list.path.segments.first() {
                if &segment.ident.to_string() == &"resource" {
                    return Ok(ResourceAttr::from_attr(list.tokens.clone().into())?);
                }
            }
        }
    }
    Ok(ResourceAttr::default())
}

fusen_attr! {
    ResourceAttr,
    id,
    path,
    method
}

fusen_attr! {
    FusenAttr,
    package,
    version
}
