#![warn(missing_docs)]
//! Derive macros for safe, redacted configuration debug output.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{format_ident, quote};
use syn::{Meta, parse::Parser};

mod debug;

#[derive(Default)]
struct StrategyAttr {
    ignore: Option<String>,
    limit: Option<String>,
    mask: Option<String>,
}

impl StrategyAttr {
    fn from_attr(args: TokenStream) -> Result<Self, syn::Error> {
        let args =
            syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated.parse(args)?;
        let mut value = Self::default();
        for argument in args {
            match argument {
                Meta::Path(path) if path.is_ident("ignore") => value.ignore = Some(String::new()),
                Meta::Path(path) if path.is_ident("mask") => value.mask = Some(String::new()),
                Meta::NameValue(field) if field.path.is_ident("limit") => {
                    let syn::Expr::Lit(syn::ExprLit { lit, .. }) = field.value else {
                        return Err(syn::Error::new_spanned(
                            field,
                            "strategy limit must be an integer literal",
                        ));
                    };
                    value.limit = Some(match lit {
                        syn::Lit::Int(value) => value.base10_digits().to_owned(),
                        syn::Lit::Str(value) => value.value(),
                        _ => {
                            return Err(syn::Error::new_spanned(
                                lit,
                                "strategy limit must be an integer literal",
                            ));
                        }
                    });
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "expected `ignore`, `mask`, or `limit = N`",
                    ));
                }
            }
        }
        Ok(value)
    }
}

#[proc_macro_derive(StrategyDebug, attributes(strategy))]
/// Derives `Debug` with per-field mask, limit, and ignore strategies.
pub fn strategy_debug(item: TokenStream) -> TokenStream {
    debug::debug(item, config_crate_path())
}

fn config_crate_path() -> proc_macro2::TokenStream {
    match crate_name("fusen-config") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{}", name.replace('-', "_"));
            quote!(::#ident)
        }
        Err(_) => quote!(::fusen_config),
    }
}
