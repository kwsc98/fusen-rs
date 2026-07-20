#![warn(missing_docs)]
//! Procedural macros for declaring fusen-rs services and handlers.

use fusen_derive_macro::fusen_attr;
use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{format_ident, quote};
use syn::{Attribute, Meta};

mod handler_macro;
mod service_macro;
mod trait_macro;

fn fusen_crate_path() -> proc_macro2::TokenStream {
    match crate_name("fusen-rs") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{}", name.replace('-', "_"));
            quote!(::#ident)
        }
        Err(_) => quote!(::fusen_rs),
    }
}

#[proc_macro_attribute]
/// Generates an internal adapter for an Aspect or LoadBalance implementation.
pub fn handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = HandlerAttr::from_attr(attr);
    match attr {
        Ok(attr) => handler_macro::fusen_handler(attr, item),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro_attribute]
/// Generates an RPC trait and strongly typed client.
pub fn fusen_trait(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = FusenAttr::from_attr(attr);
    match attr {
        Ok(attr) => trait_macro::fusen_trait(attr, item),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro_attribute]
/// Generates the server dispatch adapter for a trait implementation.
pub fn fusen_service(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = FusenAttr::from_attr(attr);
    match attr {
        Ok(attr) => service_macro::fusen_service(attr, item),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro_attribute]
/// Declares an HTTP path and method consumed by the surrounding RPC macro.
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
