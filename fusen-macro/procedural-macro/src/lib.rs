#![warn(missing_docs)]
//! Procedural macros for declaring clean-slate Fusen 0.9 interfaces.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{format_ident, quote};
use syn::parse_macro_input;

mod args;
mod sensitive;
mod sensitive_derive;
mod service_macro;
mod validate;

use args::{MethodArgs, ServiceArgs};

/// Declares one versioned service interface and generates its client and server wrapper.
///
/// The annotated trait must be non-generic and contain only ordinary `async` methods with an
/// immutable `&self` receiver. Methods take owned, named parameters and return exactly
/// `Result<Response<T>, Error>`. Wire parameters and successful response values must implement
/// the Serde and `SensitiveFields` contracts required in both client and server directions; values
/// captured by generated futures must also be `Send`.
///
/// Every method needs one [`method`] attribute. Parameters may use `#[param(path)]`,
/// `#[param(query)]`, `#[param(header)]`, `#[param(cookie)]`, `#[param(body_field)]`,
/// `#[param(body)]`, `#[param(query_map)]`, `#[param(header_map)]`, or `#[param(context)]`.
/// `#[param(name = "...")]` changes a named wire parameter and
/// `#[param(query, repeated)]` declares repeated query keys.
///
/// The expansion defines `TraitNameClient` and `TraitNameServer<T>`. Generated code uses only the
/// versioned runtime macro ABI and supports a renamed `fusen-rs` dependency.
#[proc_macro_attribute]
pub fn interface(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as ServiceArgs);
    service_macro::expand(args, item)
}

/// Declares the HTTP operation required by a method inside [`interface`].
///
/// The required fields are `method = "..."` and `path = "/..."`. Optional `consumes` and
/// `produces` fields each accept one MIME media type and default to `application/json`. Supported
/// methods are GET, POST, PUT, PATCH, DELETE, HEAD, and OPTIONS. GET, HEAD, and OPTIONS reject JSON
/// body and body-field parameters; HEAD additionally requires `Response<()>`.
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

/// Derives a process-local schema used to construct policy-controlled safe projections.
///
/// Fields recurse through their own `SensitiveFields` implementation by default. Use
/// `#[sensitive(kind = "...")]` to classify a complete serialized field or
/// `#[sensitive(opaque)]` to omit it without requiring its type to implement the trait.
/// Generic bounds are inferred for ordinary field types and unqualified recursive types. Because
/// a procedural macro cannot resolve the call site's module paths, qualified recursive paths and
/// recursive type aliases should use a complete type-level `#[sensitive(bound = "...")]`
/// override, which replaces all inferred bounds.
#[proc_macro_derive(SensitiveFields, attributes(sensitive, serde))]
pub fn derive_sensitive_fields(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::DeriveInput);
    match sensitive_derive::expand(input, &sensitivity_contract_path()) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
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

fn procedural_macro_crate_name() -> String {
    match crate_name("fusen-procedural-macro") {
        Ok(FoundCrate::Itself) => "fusen_procedural_macro".to_owned(),
        Ok(FoundCrate::Name(name)) => name.replace('-', "_"),
        Err(_) => "fusen_procedural_macro".to_owned(),
    }
}

fn sensitivity_contract_path() -> proc_macro2::TokenStream {
    match crate_name("fusen-contract") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{}", name.replace('-', "_"));
            quote!(::#ident)
        }
        Err(_) => match crate_name("fusen-rs") {
            Ok(FoundCrate::Itself) => quote!(crate::contract),
            Ok(FoundCrate::Name(name)) => {
                let ident = format_ident!("{}", name.replace('-', "_"));
                quote!(::#ident::contract)
            }
            Err(_) => quote!(::fusen_rs::contract),
        },
    }
}
