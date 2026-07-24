use syn::{Expr, ExprLit, ExprPath, Lit};

/// Parses an attribute value that must be a non-empty string literal.
pub fn parse_string(expr: &Expr, field: &str) -> Result<String, syn::Error> {
    let Expr::Lit(ExprLit {
        lit: Lit::Str(value),
        ..
    }) = expr
    else {
        return Err(syn::Error::new_spanned(
            expr,
            format!("attribute `{field}` must be a string literal"),
        ));
    };
    non_empty(value.value(), expr, field)
}

/// Parses a non-empty string literal or one unqualified identifier.
pub fn parse_ident_or_string(expr: &Expr, field: &str) -> Result<String, syn::Error> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) => non_empty(value.value(), expr, field),
        Expr::Path(ExprPath {
            qself: None, path, ..
        }) if path.segments.len() == 1 => {
            non_empty(path.segments[0].ident.to_string(), expr, field)
        }
        _ => Err(syn::Error::new_spanned(
            expr,
            format!("attribute `{field}` must be a string literal or identifier"),
        )),
    }
}

fn non_empty(value: String, expr: &Expr, field: &str) -> Result<String, syn::Error> {
    if value.trim().is_empty() {
        Err(syn::Error::new_spanned(
            expr,
            format!("attribute `{field}` must not be empty"),
        ))
    } else {
        Ok(value)
    }
}

#[macro_export]
macro_rules! fusen_attr {
    (
        $name:ident {
            $($field:ident : $kind:ident),* $(,)?
        }
    ) => {
        #[derive(Default)]
        struct $name {
            $(
                $field: Option<String>,
            )*
        }

        impl $name {
            fn from_attr(args: TokenStream) -> Result<Self, syn::Error> {
                Self::from_tokens(args.into())
            }

            fn from_tokens(args: proc_macro2::TokenStream) -> Result<Self, syn::Error> {
                use syn::parse::Parser;
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated
                    .parse2(args)
                    .and_then(Self::build_attr)
            }

            fn build_attr(
                args: syn::punctuated::Punctuated<syn::Meta, syn::Token![,]>,
            ) -> Result<Self, syn::Error> {
                $(
                    let mut $field = None;
                )*
                let expected = [$(stringify!($field)),*]
                    .map(|field| format!("`{field}`"))
                    .join(", ");

                for arg in args {
                    let syn::Meta::NameValue(name_value) = arg else {
                        return Err(syn::Error::new_spanned(
                            arg,
                            "attribute fields must use `name = value` syntax",
                        ));
                    };
                    let key = name_value.path.get_ident().ok_or_else(|| {
                        syn::Error::new_spanned(
                            &name_value.path,
                            "attribute field names must be unqualified identifiers",
                        )
                    })?;
                    let key_text = key.to_string();
                    match key_text.as_str() {
                        $(
                            stringify!($field) => {
                                if $field.is_some() {
                                    return Err(syn::Error::new_spanned(
                                        &name_value,
                                        format!("duplicate attribute field `{}`", stringify!($field)),
                                    ));
                                }
                                $field = Some($crate::fusen_attr!(
                                    @parse $kind,
                                    &name_value.value,
                                    stringify!($field)
                                )?);
                            }
                        )*
                        _ => {
                            return Err(syn::Error::new_spanned(
                                &name_value,
                                format!(
                                    "unknown attribute field `{key_text}`; expected one of: {expected}",
                                ),
                            ));
                        }
                    }
                }

                Ok(Self {
                    $($field),*
                })
            }
        }
    };

    (@parse string, $expr:expr, $field:expr) => {
        $crate::attr_macro::parse_string($expr, $field)
    };
    (@parse ident_or_string, $expr:expr, $field:expr) => {
        $crate::attr_macro::parse_ident_or_string($expr, $field)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_literals_preserve_escapes_and_raw_contents() {
        let escaped: Expr = syn::parse_str(r#""a\"b""#).unwrap();
        assert_eq!(parse_string(&escaped, "id").unwrap(), "a\"b");

        let raw: Expr = syn::parse_str("r#\"a\"b\"#").unwrap();
        assert_eq!(parse_string(&raw, "id").unwrap(), "a\"b");
    }

    #[test]
    fn identifier_values_are_structured_and_non_empty() {
        let ident: Expr = syn::parse_str("GET").unwrap();
        assert_eq!(parse_ident_or_string(&ident, "method").unwrap(), "GET");

        let expression: Expr = syn::parse_str("make_method()").unwrap();
        assert!(parse_ident_or_string(&expression, "method").is_err());
    }
}
