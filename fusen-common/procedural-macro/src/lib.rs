#![warn(missing_docs)]
//! Derive macros for safe, redacted debug output.

use fusen_common_derive_macro::fusen_attr;
use proc_macro::TokenStream;

mod debug;

#[proc_macro_derive(StrategyDebug, attributes(strategy))]
/// Derives `Debug` with per-field mask, limit, and ignore strategies.
pub fn strategy_debug(item: TokenStream) -> TokenStream {
    debug::debug(item)
}

fusen_attr! { StrategyAttr, ignore, limit, mask }
