#![warn(missing_docs)]
//! Procedural macros for declaring fusen-rs services.

use fusen_macro_support::fusen_attr;
use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{format_ident, quote};
use syn::{Attribute, Meta, Path};

mod service_macro;
mod trait_macro;
mod validate;

fn fusen_crate_path() -> proc_macro2::TokenStream {
    match crate_name("fusen-rs") {
        Ok(FoundCrate::Itself) => quote!(::fusen_rs),
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{}", name.replace('-', "_"));
            quote!(::#ident)
        }
        Err(_) => quote!(::fusen_rs),
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

fn get_asset_by_attrs(attrs: &[Attribute]) -> Result<ResourceAttr, syn::Error> {
    let mut resource = None;
    for attr in attrs {
        if !is_asset_attr(attr) {
            continue;
        }
        if resource.is_some() {
            return Err(syn::Error::new_spanned(
                attr,
                "only one `asset` attribute is allowed",
            ));
        }
        let Meta::List(list) = &attr.meta else {
            return Err(syn::Error::new_spanned(
                attr,
                "`asset` requires `path = ...` and/or `method = ...` arguments",
            ));
        };
        resource = Some(ResourceAttr::from_tokens(list.tokens.clone())?);
    }
    Ok(resource.unwrap_or_default())
}

fn is_asset_attr(attr: &Attribute) -> bool {
    match &attr.meta {
        Meta::Path(path) => is_asset_path(path),
        Meta::List(list) => is_asset_path(&list.path),
        Meta::NameValue(value) => is_asset_path(&value.path),
    }
}

fn is_asset_path(path: &Path) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| segment.ident == "asset")
}

fusen_attr! {
    ResourceAttr {
        path: string,
        method: ident_or_string,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::{ItemTrait, parse::Parser};

    #[test]
    fn qualified_asset_is_parsed() {
        let item: ItemTrait = syn::parse_quote! {
            #[renamed::asset(path = r#"/quoted/\"value"#, method = "get")]
            trait Demo {
                async fn call(&self);
            }
        };
        let resource = get_asset_by_attrs(&item.attrs).unwrap();
        assert_eq!(resource.path.as_deref(), Some("/quoted/\\\"value"));
        assert_eq!(resource.method.as_deref(), Some("get"));
    }

    #[test]
    fn duplicate_fields_and_assets_are_rejected() {
        let args = syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated
            .parse2(quote!(path = "/one", path = "/two"))
            .unwrap();
        assert!(ResourceAttr::build_attr(args).is_err());

        let item: ItemTrait = syn::parse_quote! {
            #[asset(path = "/one")]
            #[renamed::asset(path = "/two")]
            trait Demo {
                async fn call(&self);
            }
        };
        assert!(get_asset_by_attrs(&item.attrs).is_err());
    }
}

fusen_attr! {
    FusenAttr {
        id: string,
        version: string,
        group: string,
    }
}
