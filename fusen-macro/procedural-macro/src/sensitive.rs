//! Shared parsing for sensitivity declarations consumed by generated APIs.

use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Attribute, LitStr, Meta, Token, WherePredicate};

const MAX_KIND_BYTES: usize = 64;

/// A sensitivity override attached to a type, DTO field, or service invocation parameter.
#[derive(Clone)]
pub(crate) enum SensitiveOverride {
    /// Classifies the complete represented value with the named kind.
    Kind(LitStr),
    /// Omits the complete represented value from safe projections.
    Opaque,
}

/// Type-level sensitivity settings consumed by the derive macro.
pub(crate) struct SensitiveContainerAttrs {
    /// Optional classification of the complete value.
    pub(crate) value: Option<SensitiveOverride>,
    /// Explicit bounds replacing bounds inferred from recursively shaped fields.
    pub(crate) bounds: Option<Vec<WherePredicate>>,
}

/// Returns whether `attribute` is the sensitivity helper attribute.
pub(crate) fn is_sensitive_attr(attribute: &Attribute) -> bool {
    attribute.path().is_ident("sensitive")
}

/// Parses at most one sensitivity override from a list of attributes.
pub(crate) fn parse_sensitive_attrs(
    attributes: &[Attribute],
) -> syn::Result<Option<SensitiveOverride>> {
    parse_sensitive_attrs_impl(attributes, false).map(|attributes| attributes.value)
}

/// Parses type-level sensitivity settings, including an optional explicit bound override.
pub(crate) fn parse_sensitive_container_attrs(
    attributes: &[Attribute],
) -> syn::Result<SensitiveContainerAttrs> {
    parse_sensitive_attrs_impl(attributes, true)
}

fn parse_sensitive_attrs_impl(
    attributes: &[Attribute],
    allow_bounds: bool,
) -> syn::Result<SensitiveContainerAttrs> {
    let mut value = None;
    let mut value_span = None;
    let mut bounds = None;
    let mut bounds_span = None;
    let mut saw_attribute = false;

    for attribute in attributes
        .iter()
        .filter(|attribute| is_sensitive_attr(attribute))
    {
        saw_attribute = true;
        let Meta::List(_) = &attribute.meta else {
            return Err(syn::Error::new_spanned(
                attribute,
                if allow_bounds {
                    "`sensitive` must use `#[sensitive(kind = \"...\")]`, `#[sensitive(opaque)]`, or `#[sensitive(bound = \"...\")]` syntax"
                } else {
                    "`sensitive` must use `#[sensitive(kind = \"...\")]` or `#[sensitive(opaque)]` syntax"
                },
            ));
        };

        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("kind") {
                if let Some(first_span) = value_span {
                    let mut error =
                        meta.error("a value may declare only one sensitivity kind or `opaque`");
                    error.combine(syn::Error::new(
                        first_span,
                        "first sensitivity declaration here",
                    ));
                    return Err(error);
                }
                if let Some(first_span) = bounds_span {
                    let mut error = meta.error(
                        "a complete-value sensitivity declaration cannot be combined with `bound`",
                    );
                    error.combine(syn::Error::new(first_span, "bound override declared here"));
                    return Err(error);
                }
                let kind = meta.value()?.parse::<LitStr>()?;
                validate_kind(&kind)?;
                value = Some(SensitiveOverride::Kind(kind));
                value_span = Some(meta.path.span());
                Ok(())
            } else if meta.path.is_ident("opaque") {
                if !meta.input.is_empty() {
                    return Err(meta.error("`opaque` does not accept a value"));
                }
                if let Some(first_span) = value_span {
                    let mut error =
                        meta.error("a value may declare only one sensitivity kind or `opaque`");
                    error.combine(syn::Error::new(
                        first_span,
                        "first sensitivity declaration here",
                    ));
                    return Err(error);
                }
                if let Some(first_span) = bounds_span {
                    let mut error = meta.error(
                        "a complete-value sensitivity declaration cannot be combined with `bound`",
                    );
                    error.combine(syn::Error::new(first_span, "bound override declared here"));
                    return Err(error);
                }
                value = Some(SensitiveOverride::Opaque);
                value_span = Some(meta.path.span());
                Ok(())
            } else if meta.path.is_ident("bound") {
                if !allow_bounds {
                    return Err(meta.error(
                        "`bound` is supported only on a type deriving `SensitiveFields`",
                    ));
                }
                if let Some(first_span) = bounds_span {
                    let mut error = meta.error("duplicate sensitivity bound override");
                    error.combine(syn::Error::new(first_span, "first bound override here"));
                    return Err(error);
                }
                if let Some(first_span) = value_span {
                    let mut error = meta.error(
                        "`bound` cannot be combined with a complete-value sensitivity declaration",
                    );
                    error.combine(syn::Error::new(
                        first_span,
                        "complete-value sensitivity declared here",
                    ));
                    return Err(error);
                }
                let bound = meta.value()?.parse::<LitStr>()?;
                let parser = Punctuated::<WherePredicate, Token![,]>::parse_terminated;
                let parsed = parser.parse_str(&bound.value()).map_err(|error| {
                    syn::Error::new_spanned(
                        &bound,
                        format!(
                            "invalid sensitivity bound; expected comma-separated where predicates: {error}"
                        ),
                    )
                })?;
                bounds = Some(parsed.into_iter().collect());
                bounds_span = Some(meta.path.span());
                Ok(())
            } else {
                let expected = if allow_bounds {
                    "unknown sensitive field; expected `kind`, `opaque`, or `bound`"
                } else {
                    "unknown sensitive field; expected `kind` or `opaque`"
                };
                Err(meta.error(expected))
            }
        })?;
    }

    if saw_attribute && value.is_none() && bounds.is_none() {
        return Err(syn::Error::new_spanned(
            attributes
                .iter()
                .find(|attribute| is_sensitive_attr(attribute))
                .expect("the sensitivity attribute was observed"),
            if allow_bounds {
                "`sensitive` requires `kind = \"...\"`, `opaque`, or `bound = \"...\"`"
            } else {
                "`sensitive` requires `kind = \"...\"` or `opaque`"
            },
        ));
    }

    Ok(SensitiveContainerAttrs { value, bounds })
}

fn validate_kind(kind: &LitStr) -> syn::Result<()> {
    let value = kind.value();
    if value.is_empty()
        || value.len() > MAX_KIND_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(syn::Error::new_spanned(
            kind,
            "sensitivity kind must contain 1-64 ASCII letters, digits, '.', '_' or '-'",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::DeriveInput;

    fn attributes(tokens: proc_macro2::TokenStream) -> Vec<Attribute> {
        syn::parse2::<DeriveInput>(tokens).unwrap().attrs
    }

    #[test]
    fn parses_kind_and_opaque() {
        let kind = attributes(quote! {
            #[sensitive(kind = "tenant.identifier")]
            struct Request;
        });
        assert!(matches!(
            parse_sensitive_attrs(&kind).unwrap(),
            Some(SensitiveOverride::Kind(kind)) if kind.value() == "tenant.identifier"
        ));

        let opaque = attributes(quote! {
            #[sensitive(opaque)]
            struct Request;
        });
        assert!(matches!(
            parse_sensitive_attrs(&opaque).unwrap(),
            Some(SensitiveOverride::Opaque)
        ));
    }

    #[test]
    fn parses_type_level_bound_overrides() {
        let attrs = attributes(quote! {
            #[sensitive(bound = "T: SensitiveFields, Wrapper<T>: SensitiveFields")]
            struct Request<T>(T);
        });
        let parsed = parse_sensitive_container_attrs(&attrs).unwrap();
        assert!(parsed.value.is_none());
        assert_eq!(parsed.bounds.unwrap().len(), 2);

        let empty = attributes(quote! {
            #[sensitive(bound = "")]
            struct Request<T>(T);
        });
        assert!(
            parse_sensitive_container_attrs(&empty)
                .unwrap()
                .bounds
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn rejects_conflicting_or_invalid_declarations() {
        let conflict = attributes(quote! {
            #[sensitive(kind = "secret", opaque)]
            struct Request;
        });
        assert!(parse_sensitive_attrs(&conflict).is_err());

        let invalid = attributes(quote! {
            #[sensitive(kind = "not a token")]
            struct Request;
        });
        assert!(parse_sensitive_attrs(&invalid).is_err());

        let empty = attributes(quote! {
            #[sensitive()]
            struct Request;
        });
        assert!(parse_sensitive_attrs(&empty).is_err());

        let field_bound = attributes(quote! {
            #[sensitive(bound = "T: SensitiveFields")]
            struct Request<T>(T);
        });
        assert!(parse_sensitive_attrs(&field_bound).is_err());

        let classified_bound = attributes(quote! {
            #[sensitive(kind = "secret", bound = "T: SensitiveFields")]
            struct Request<T>(T);
        });
        let error = parse_sensitive_container_attrs(&classified_bound)
            .err()
            .expect("classification and bound must conflict");
        assert!(error.to_string().contains("cannot be combined"));
    }
}
