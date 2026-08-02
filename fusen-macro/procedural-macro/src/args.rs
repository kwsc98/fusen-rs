//! Structured parsers for the interface and method attributes.

use syn::parse::{Parse, ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::{Expr, ExprLit, Lit, LitStr, Meta, Token};

#[derive(Default)]
pub(crate) struct ServiceArgs {
    pub(crate) name: Option<LitStr>,
    pub(crate) group: Option<LitStr>,
    pub(crate) version: Option<LitStr>,
}

impl Parse for ServiceArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let fields = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        let mut args = Self::default();
        for field in fields {
            let Meta::NameValue(field) = field else {
                return Err(syn::Error::new_spanned(
                    field,
                    "interface fields must use `name = \"value\"` syntax",
                ));
            };
            let Some(name) = field.path.get_ident() else {
                return Err(syn::Error::new_spanned(
                    field.path,
                    "interface field names must be unqualified identifiers",
                ));
            };
            match name.to_string().as_str() {
                "name" => set_once(
                    &mut args.name,
                    parse_string(field.value.clone(), "name")?,
                    &field,
                    "name",
                )?,
                "group" => set_once(
                    &mut args.group,
                    parse_string(field.value.clone(), "group")?,
                    &field,
                    "group",
                )?,
                "version" => set_once(
                    &mut args.version,
                    parse_string(field.value.clone(), "version")?,
                    &field,
                    "version",
                )?,
                unknown => {
                    return Err(syn::Error::new_spanned(
                        field,
                        format!(
                            "unknown interface field `{unknown}`; expected `name`, `group`, or `version`"
                        ),
                    ));
                }
            }
        }
        Ok(args)
    }
}

#[derive(Default)]
pub(crate) struct MethodArgs {
    pub(crate) method: Option<LitStr>,
    pub(crate) path: Option<LitStr>,
    pub(crate) consumes: Option<LitStr>,
    pub(crate) produces: Option<LitStr>,
}

impl MethodArgs {
    pub(crate) fn parse_tokens(tokens: proc_macro2::TokenStream) -> syn::Result<Self> {
        Punctuated::<Meta, Token![,]>::parse_terminated
            .parse2(tokens)
            .and_then(Self::from_fields)
    }

    fn from_fields(fields: Punctuated<Meta, Token![,]>) -> syn::Result<Self> {
        let mut args = Self::default();
        for field in fields {
            let Meta::NameValue(field) = field else {
                return Err(syn::Error::new_spanned(
                    field,
                    "method fields must use `name = value` syntax",
                ));
            };
            let Some(name) = field.path.get_ident() else {
                return Err(syn::Error::new_spanned(
                    field.path,
                    "method field names must be unqualified identifiers",
                ));
            };
            match name.to_string().as_str() {
                "method" => set_once(
                    &mut args.method,
                    parse_string(field.value.clone(), "method")?,
                    &field,
                    "method",
                )?,
                "path" => set_once(
                    &mut args.path,
                    parse_string(field.value.clone(), "path")?,
                    &field,
                    "path",
                )?,
                "consumes" => set_once(
                    &mut args.consumes,
                    parse_string(field.value.clone(), "consumes")?,
                    &field,
                    "consumes",
                )?,
                "produces" => set_once(
                    &mut args.produces,
                    parse_string(field.value.clone(), "produces")?,
                    &field,
                    "produces",
                )?,
                unknown => {
                    return Err(syn::Error::new_spanned(
                        field,
                        format!(
                            "unknown method field `{unknown}`; expected `method`, `path`, `consumes`, or `produces`"
                        ),
                    ));
                }
            }
        }
        Ok(args)
    }
}

fn parse_string(value: Expr, field: &str) -> syn::Result<LitStr> {
    let Expr::Lit(ExprLit {
        lit: Lit::Str(value),
        ..
    }) = value
    else {
        return Err(syn::Error::new_spanned(
            value,
            format!("`{field}` must be a string literal"),
        ));
    };
    if value.value().is_empty() {
        return Err(syn::Error::new_spanned(
            value,
            format!("`{field}` must not be empty"),
        ));
    }
    Ok(value)
}

fn set_once<T>(
    slot: &mut Option<T>,
    value: T,
    field: &impl quote::ToTokens,
    field_name: &str,
) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new_spanned(
            field,
            format!("duplicate attribute field `{field_name}`"),
        ));
    }
    *slot = Some(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn parses_service_contract() {
        let args: ServiceArgs =
            syn::parse2(quote!(name = "user", group = "prod", version = "1")).unwrap();
        assert_eq!(args.name.unwrap().value(), "user");
        assert_eq!(args.group.unwrap().value(), "prod");
        assert_eq!(args.version.unwrap().value(), "1");
    }

    #[test]
    fn parses_method_contract() {
        let args = MethodArgs::parse_tokens(quote!(
            method = "GET",
            path = "/users/{id}",
            consumes = "application/json",
            produces = "application/problem+json"
        ))
        .unwrap();
        assert_eq!(args.method.unwrap().value(), "GET");
        assert_eq!(args.path.unwrap().value(), "/users/{id}");
        assert_eq!(args.consumes.unwrap().value(), "application/json");
        assert_eq!(args.produces.unwrap().value(), "application/problem+json");
    }

    #[test]
    fn rejects_unstructured_values_and_duplicates() {
        assert!(syn::parse2::<ServiceArgs>(quote!(name = concat!("u", "ser"))).is_err());
        let duplicate = MethodArgs::parse_tokens(quote!(method = "GET", method = "POST"))
            .err()
            .expect("duplicate method fields must fail");
        assert_eq!(duplicate.to_string(), "duplicate attribute field `method`");
        assert!(MethodArgs::parse_tokens(quote!(query = ["id"])).is_err());
    }
}
