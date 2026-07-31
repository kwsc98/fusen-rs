//! Shared parsing for sensitivity declarations consumed by generated APIs.

use syn::{Attribute, LitStr, Meta};

const MAX_KIND_BYTES: usize = 64;

/// A sensitivity override attached to a type, DTO field, or RPC parameter.
#[derive(Clone)]
pub(crate) enum SensitiveOverride {
    /// Classifies the complete serialized value with the named kind.
    Kind(LitStr),
    /// Omits the complete serialized value from safe projections.
    Opaque,
}

/// Returns whether `attribute` is the sensitivity helper attribute.
pub(crate) fn is_sensitive_attr(attribute: &Attribute) -> bool {
    attribute.path().is_ident("sensitive")
}

/// Parses at most one sensitivity override from a list of attributes.
pub(crate) fn parse_sensitive_attrs(
    attributes: &[Attribute],
) -> syn::Result<Option<SensitiveOverride>> {
    let mut value = None;
    let mut saw_attribute = false;

    for attribute in attributes
        .iter()
        .filter(|attribute| is_sensitive_attr(attribute))
    {
        saw_attribute = true;
        let Meta::List(_) = &attribute.meta else {
            return Err(syn::Error::new_spanned(
                attribute,
                "`sensitive` must use `#[sensitive(kind = \"...\")]` or `#[sensitive(opaque)]` syntax",
            ));
        };

        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("kind") {
                if value.is_some() {
                    return Err(
                        meta.error("a value may declare only one sensitivity kind or `opaque`")
                    );
                }
                let kind = meta.value()?.parse::<LitStr>()?;
                validate_kind(&kind)?;
                value = Some(SensitiveOverride::Kind(kind));
                Ok(())
            } else if meta.path.is_ident("opaque") {
                if !meta.input.is_empty() {
                    return Err(meta.error("`opaque` does not accept a value"));
                }
                if value.is_some() {
                    return Err(
                        meta.error("a value may declare only one sensitivity kind or `opaque`")
                    );
                }
                value = Some(SensitiveOverride::Opaque);
                Ok(())
            } else {
                Err(meta.error("unknown sensitive field; expected `kind` or `opaque`"))
            }
        })?;
    }

    if saw_attribute && value.is_none() {
        return Err(syn::Error::new_spanned(
            attributes
                .iter()
                .find(|attribute| is_sensitive_attr(attribute))
                .expect("the sensitivity attribute was observed"),
            "`sensitive` requires `kind = \"...\"` or `opaque`",
        ));
    }

    Ok(value)
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
    }
}
