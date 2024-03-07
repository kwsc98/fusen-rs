use fusen_common::fusen_attr;
use proc_macro::TokenStream;
use quote::ToTokens;
use syn::{parse::Parser, Attribute, Meta};
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
