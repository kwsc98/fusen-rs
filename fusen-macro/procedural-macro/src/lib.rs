use fusen_derive_macro::fusen_attr;
use proc_macro::TokenStream;
use syn::{Attribute, Meta};

mod handler_macro;
mod service_macro;
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

#[proc_macro_attribute]
pub fn fusen_service(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = FusenAttr::from_attr(attr);
    match attr {
        Ok(attr) => service_macro::fusen_service(attr, item),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn asset(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

fn get_asset_by_attrs(attrs: &Vec<Attribute>) -> Result<ResourceAttr, syn::Error> {
    for attr in attrs {
        if let Meta::List(list) = &attr.meta
            && let Some(segment) = list.path.segments.first()
            && segment.ident == "asset"
        {
            return ResourceAttr::from_attr(list.tokens.clone().into());
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
