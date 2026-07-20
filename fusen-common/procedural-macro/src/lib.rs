#![warn(missing_docs)]
//! Derive macros for safe, redacted debug output.

use fusen_common_derive_macro::fusen_attr;
use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{format_ident, quote};

mod debug;

#[proc_macro_derive(StrategyDebug, attributes(strategy))]
/// Derives `Debug` with per-field mask, limit, and ignore strategies.
pub fn strategy_debug(item: TokenStream) -> TokenStream {
    debug::debug(item, common_crate_path())
}

fn common_crate_path() -> proc_macro2::TokenStream {
    match crate_name("fusen-common") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{}", name.replace('-', "_"));
            quote!(::#ident)
        }
        Err(_) => quote!(::fusen_common),
    }
}

fusen_attr! { StrategyAttr, ignore, limit, mask }
