use proc_macro::TokenStream;
use std::collections::HashMap;
use syn::{parse::Parser, Attribute, Meta};
use quote::{quote, ToTokens};
mod server_macro;
mod trait_macro;

#[proc_macro_attribute]
pub fn fusen_trait(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = get_trait_info("fusen_trait",attr);
    match attr {
        Ok(attr) => trait_macro::fusen_trait(attr.0,attr.1, item),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn fusen_server(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = get_trait_info("fusen_server",attr);
    match attr {
        Ok(attr) => server_macro::fusen_server(attr.0,attr.1, item),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn resource(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

fn parse_attr(attr: TokenStream) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let attr = attr.clone().to_string();
    let args: Vec<&str> = attr.split(",").collect();
    for arg in args {
        let arg = arg.replace(" ", "");
        let item: Vec<&str> = arg.split("=").collect();
        map.insert(
            item[0].to_string().clone(),
            item[1].replace("\"", "").to_string().clone(),
        );
    }
    return map;
}

fn get_resource_by_attrs(attrs: &Vec<Attribute>) -> (Option<String>, Option<String>) {
    let mut parent_path = None;
    let mut parent_method = None;
    for attr in attrs {
        if let Meta::List(list) = &attr.meta {
            if let Some(segment) = list.path.segments.first() {
                if &segment.ident.to_string() == &"resource" {
                    let temp_map = parse_attr(list.tokens.clone().into());
                    if let Some(path) = temp_map.get("path") {
                        let _ = parent_path.insert(path.to_string());
                    }
                    if let Some(method) = temp_map.get("method") {
                        let _ = parent_method.insert(method.to_string());
                    }
                }
            }
        }
    }
    (parent_path, parent_method)
}

type AttributeArgs = syn::punctuated::Punctuated<syn::Meta, syn::Token![,]>;

fn get_trait_info(
    macro_name: &str,
    args: TokenStream,
) -> Result<(Option<String>, Option<String>), syn::Error> {
    AttributeArgs::parse_terminated
        .parse2(args.into())
        .and_then(|args| build_config(macro_name, args))
}

fn build_config(
    macro_name: &str,
    args: AttributeArgs,
) -> Result<(Option<String>, Option<String>), syn::Error> {
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
                    syn::Expr::Lit(syn::ExprLit { lit, .. }) => lit,
                    expr => return Err(syn::Error::new_spanned(expr, "Must be a literal")),
                };
                match ident.as_str() {
                    "package" => {
                        let _ = package.insert(lit.to_token_stream().to_string().replace("\"", ""));
                    }
                    "version" => {
                        let _ = version.insert(lit.to_token_stream().to_string().replace("\"", ""));
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
