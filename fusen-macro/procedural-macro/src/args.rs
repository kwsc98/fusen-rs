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
                )?,
                "group" => set_once(
                    &mut args.group,
                    parse_string(field.value.clone(), "group")?,
                    &field,
                )?,
                "version" => set_once(
                    &mut args.version,
                    parse_string(field.value.clone(), "version")?,
                    &field,
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
    pub(crate) idempotency: Option<LitStr>,
    pub(crate) spring: Option<SpringArgs>,
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
            match field {
                Meta::NameValue(field) if field.path.is_ident("idempotency") => {
                    let value = parse_string(field.value.clone(), "idempotency")?;
                    set_once(&mut args.idempotency, value, &field)?;
                }
                Meta::List(field) if field.path.is_ident("spring") => {
                    let value = SpringArgs::parse_tokens(field.tokens.clone())?;
                    set_once(&mut args.spring, value, &field)?;
                }
                Meta::Path(path) | Meta::List(syn::MetaList { path, .. })
                    if path.is_ident("idempotency") =>
                {
                    return Err(syn::Error::new_spanned(
                        path,
                        "`idempotency` must use `idempotency = \"none|idempotent|safe\"` syntax",
                    ));
                }
                Meta::Path(path) | Meta::NameValue(syn::MetaNameValue { path, .. })
                    if path.is_ident("spring") =>
                {
                    return Err(syn::Error::new_spanned(
                        path,
                        "`spring` must use `spring(method = ..., path = ..., ...)` syntax",
                    ));
                }
                unknown => {
                    return Err(syn::Error::new_spanned(
                        unknown,
                        "unknown method field; expected `idempotency` or `spring(...)`",
                    ));
                }
            }
        }
        Ok(args)
    }
}

#[derive(Default)]
pub(crate) struct SpringArgs {
    pub(crate) method: Option<LitStr>,
    pub(crate) path: Option<LitStr>,
}

impl SpringArgs {
    fn parse_tokens(tokens: proc_macro2::TokenStream) -> syn::Result<Self> {
        let fields = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(tokens)?;
        let mut args = Self::default();
        for field in fields {
            let Meta::NameValue(field) = field else {
                return Err(syn::Error::new_spanned(
                    field,
                    "spring fields must use `name = value` syntax",
                ));
            };
            let Some(name) = field.path.get_ident() else {
                return Err(syn::Error::new_spanned(
                    field.path,
                    "spring field names must be unqualified identifiers",
                ));
            };
            match name.to_string().as_str() {
                "method" => set_once(
                    &mut args.method,
                    parse_string(field.value.clone(), "spring.method")?,
                    &field,
                )?,
                "path" => set_once(
                    &mut args.path,
                    parse_string(field.value.clone(), "spring.path")?,
                    &field,
                )?,
                unknown => {
                    return Err(syn::Error::new_spanned(
                        field,
                        format!(
                            "unknown spring field `{unknown}`; expected `method` or `path`; field roles belong on RpcMessage fields"
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

fn set_once<T>(slot: &mut Option<T>, value: T, field: &impl quote::ToTokens) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new_spanned(field, "duplicate attribute field"));
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
    fn parses_nested_spring_contract() {
        let args = MethodArgs::parse_tokens(quote!(
            idempotency = "safe",
            spring(method = "GET", path = "/users/{id}")
        ))
        .unwrap();
        assert_eq!(args.idempotency.unwrap().value(), "safe");
        let spring = args.spring.unwrap();
        assert_eq!(spring.method.unwrap().value(), "GET");
        assert_eq!(spring.path.unwrap().value(), "/users/{id}");
    }

    #[test]
    fn rejects_unstructured_values_and_duplicates() {
        assert!(syn::parse2::<ServiceArgs>(quote!(name = concat!("u", "ser"))).is_err());
        assert!(MethodArgs::parse_tokens(quote!(spring(method = "GET", method = "POST"))).is_err());
        assert!(MethodArgs::parse_tokens(quote!(spring(query = ["id"]))).is_err());
    }
}
