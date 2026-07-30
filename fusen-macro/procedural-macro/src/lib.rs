#![warn(missing_docs)]
//! Procedural macros for declaring clean-slate Fusen 0.9 interfaces.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{format_ident, quote};
use syn::parse_macro_input;

mod args;
mod service_macro;
mod validate;

use args::{MethodArgs, ServiceArgs};

/// Declares one versioned RPC interface and generates its client and server wrapper.
#[proc_macro_attribute]
pub fn interface(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as ServiceArgs);
    service_macro::expand(args, item)
}

/// Carries method metadata consumed by the surrounding [`interface`] macro.
#[proc_macro_attribute]
pub fn method(attr: TokenStream, item: TokenStream) -> TokenStream {
    match MethodArgs::parse_tokens(attr.into()) {
        Ok(_) => {
            let error = syn::Error::new(
                proc_macro2::Span::call_site(),
                "`#[method]` may only be used on a method inside an `#[interface]` trait",
            )
            .into_compile_error();
            let item: proc_macro2::TokenStream = item.into();
            quote!(#error #item).into()
        }
        Err(error) => {
            let error = error.into_compile_error();
            let item: proc_macro2::TokenStream = item.into();
            quote!(#error #item).into()
        }
    }
}

fn runtime_path() -> proc_macro2::TokenStream {
    match crate_name("fusen-rs") {
        Ok(FoundCrate::Itself) => quote!(::fusen_rs),
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{}", name.replace('-', "_"));
            quote!(::#ident)
        }
        Err(_) => quote!(::fusen_rs),
    }
}

fn runtime_crate_name() -> String {
    match crate_name("fusen-rs") {
        Ok(FoundCrate::Itself) => "fusen_rs".to_owned(),
        Ok(FoundCrate::Name(name)) => name.replace('-', "_"),
        Err(_) => "fusen_rs".to_owned(),
    }
}
