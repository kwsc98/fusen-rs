use proc_macro::TokenStream;
use quote::ToTokens;
use std::collections::HashMap;
use syn::{parse::Parser, Attribute, Meta};
mod server_macro;
mod trait_macro;

#[proc_macro_attribute]
pub fn fusen_trait(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = get_trait_attrs(attr);
    match attr {
        Ok(attr) => trait_macro::fusen_trait(attr.0, attr.1, item),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn fusen_server(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = get_trait_attrs(attr);
    match attr {
        Ok(attr) => server_macro::fusen_server(attr.0, attr.1, item),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn resource(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

fn get_resource_by_attrs(
    attrs: &Vec<Attribute>,
) -> Result<(Option<String>, Option<String>), syn::Error> {
    let mut parent_path = None;
    let mut parent_method = None;
    for attr in attrs {
        if let Meta::List(list) = &attr.meta {
            if let Some(segment) = list.path.segments.first() {
                if &segment.ident.to_string() == &"resource" {
                    let temp_map = get_resource_attrs(list.tokens.clone().into())?;
                    if let Some(path) = temp_map.0 {
                        let _ = parent_path.insert(path.to_string());
                    }
                    if let Some(method) = temp_map.1 {
                        let _ = parent_method.insert(method.to_string());
                    }
                }
            }
        }
    }
    Ok((parent_path, parent_method))
}

type AttributeArgs = syn::punctuated::Punctuated<syn::Meta, syn::Token![,]>;

fn get_trait_attrs(args: TokenStream) -> Result<(Option<String>, Option<String>), syn::Error> {
    AttributeArgs::parse_terminated
        .parse2(args.into())
        .and_then(|args| build_trait_attr(args))
}

fn get_resource_attrs(args: TokenStream) -> Result<(Option<String>, Option<String>), syn::Error> {
    AttributeArgs::parse_terminated
        .parse2(args.into())
        .and_then(|args| build_resource_attr(args))
}

fn build_trait_attr(args: AttributeArgs) -> Result<(Option<String>, Option<String>), syn::Error> {
    let mut package = None;
    let mut version = None;
    for arg in args {
        match arg {
            syn::Meta::NameValue(namevalue) => {
                let ident = namevalue
                    .path
                    .get_ident()
                    .ok_or_else(|| {
                        syn::Error::new_spanned(&namevalue, "Must have specified ident")
                    })?
                    .to_string()
                    .to_lowercase();
                let lit = match &namevalue.value {
                    syn::Expr::Lit(syn::ExprLit { lit, .. }) => lit.to_token_stream().to_string(),
                    expr => expr.to_token_stream().to_string(),
                }
                .replace("\"", "");
                match ident.as_str() {
                    "package" => {
                        let _ = package.insert(lit);
                    }
                    "version" => {
                        let _ = version.insert(lit);
                    }
                    name => {
                        let msg = format!(
                            "Unknown attribute {} is specified; expected one of: `package`, `version` ",
                            name,
                        );
                        return Err(syn::Error::new_spanned(namevalue, msg));
                    }
                }
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "Unknown attribute inside the macro",
                ));
            }
        }
    }
    Ok((package, version))
}

fn build_resource_attr(
    args: AttributeArgs,
) -> Result<(Option<String>, Option<String>), syn::Error> {
    let mut path = None;
    let mut method = None;
    for arg in args {
        match arg {
            syn::Meta::NameValue(namevalue) => {
                let ident = namevalue
                    .path
                    .get_ident()
                    .ok_or_else(|| {
                        syn::Error::new_spanned(&namevalue, "Must have specified ident")
                    })?
                    .to_string()
                    .to_lowercase();
                let lit = match &namevalue.value {
                    syn::Expr::Lit(syn::ExprLit { lit, .. }) => lit.to_token_stream().to_string(),
                    expr => expr.to_token_stream().to_string(),
                }
                .replace("\"", "");
                match ident.as_str() {
                    "path" => {
                        let _ = path.insert(lit);
                    }
                    "method" => {
                        let _ = method.insert(lit);
                    }
                    name => {
                        let msg = format!(
                            "Unknown attribute {} is specified; expected one of: `path`, `method` ",
                            name,
                        );
                        return Err(syn::Error::new_spanned(namevalue, msg));
                    }
                }
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "Unknown attribute inside the macro",
                ));
            }
        }
    }
    Ok((path, method))
}
