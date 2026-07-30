use crate::runtime_path;
use proc_macro::TokenStream;
use quote::quote;
use std::collections::BTreeSet;
use syn::{
    Data, DeriveInput, Fields, GenericArgument, LitStr, PathArguments, Type, TypePath,
    parse_macro_input,
};

pub(crate) fn expand(item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as DeriveInput);
    match expand_tokens(item) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_tokens(item: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let runtime = runtime_path();
    let abi = quote!(#runtime::__macro::v1);
    if !item.generics.params.is_empty() || item.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &item.generics,
            "RpcMessage structs must not declare generic parameters",
        ));
    }
    let Data::Struct(data) = &item.data else {
        return Err(syn::Error::new_spanned(
            &item.ident,
            "RpcMessage may only be derived for structs with named fields",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "RpcMessage may only be derived for structs with named fields; use `()` for an empty request",
        ));
    };
    if fields.named.is_empty() {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "empty RpcMessage structs are not supported; use `()` for an empty request",
        ));
    }

    let mut body = None;
    let mut names = BTreeSet::new();
    let mut schemas = Vec::with_capacity(fields.named.len());
    for field in &fields.named {
        let ident = field.ident.as_ref().expect("named fields have identifiers");
        let rust_name = ident.to_string().trim_start_matches("r#").to_owned();
        let mut source = None;
        let mut wire_name = None;
        for attribute in field
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("rpc"))
        {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("path")
                    || meta.path.is_ident("query")
                    || meta.path.is_ident("body")
                {
                    if source.is_some() {
                        return Err(meta.error(
                            "each RpcMessage field must declare exactly one of path, query, or body",
                        ));
                    }
                    source = Some(if meta.path.is_ident("path") {
                        FieldSource::Path
                    } else if meta.path.is_ident("query") {
                        FieldSource::Query
                    } else {
                        FieldSource::Body
                    });
                    Ok(())
                } else if meta.path.is_ident("name") {
                    if wire_name.is_some() {
                        return Err(meta.error("duplicate rpc name"));
                    }
                    wire_name = Some(meta.value()?.parse::<LitStr>()?);
                    Ok(())
                } else {
                    Err(meta.error("unknown rpc field; expected path, query, body, or name"))
                }
            })?;
        }
        let source = source.ok_or_else(|| {
            syn::Error::new_spanned(
                field,
                "each RpcMessage field must declare exactly one of #[rpc(path)], #[rpc(query)], or #[rpc(body)]",
            )
        })?;
        if source == FieldSource::Body && body.replace(ident).is_some() {
            return Err(syn::Error::new_spanned(
                field,
                "an RpcMessage may declare at most one #[rpc(body)] field",
            ));
        }
        let wire_name = wire_name.map_or_else(|| rust_name.clone(), |name| name.value());
        validate_name(&wire_name, field)?;
        if !names.insert(wire_name.clone()) {
            return Err(syn::Error::new_spanned(
                field,
                format!("duplicate RPC wire field name `{wire_name}`"),
            ));
        }
        let repeated = if source == FieldSource::Query {
            if direct_generic_type(&field.ty, "Option")
                .and_then(|inner| direct_generic_type(inner, "Vec"))
                .is_some()
            {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "query fields may not use Option<Vec<T>>; use Vec<T> so omission has one unambiguous empty-list meaning",
                ));
            }
            direct_generic_type(&field.ty, "Vec").is_some()
        } else {
            if direct_generic_type(&field.ty, "Vec").is_some() {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "only #[rpc(query)] fields may use Vec<T>",
                ));
            }
            false
        };
        let scalar_type = direct_generic_type(&field.ty, "Option").unwrap_or(&field.ty);
        let scalar_type = direct_generic_type(scalar_type, "Vec").unwrap_or(scalar_type);
        let parse_spring_json_primitive = is_json_primitive_type(scalar_type);
        let source = match source {
            FieldSource::Path => quote!(Path),
            FieldSource::Query => quote!(Query),
            FieldSource::Body => quote!(Body),
        };
        schemas.push(quote! {
            #abi::RpcField::new(
                #rust_name,
                #wire_name,
                #abi::RpcFieldSource::#source,
                #repeated,
                #parse_spring_json_primitive,
            )
        });
    }

    let ident = &item.ident;
    Ok(quote! {
        impl #abi::RpcMessage for #ident {
            fn fields() -> &'static [#abi::RpcField] {
                const FIELDS: &[#abi::RpcField] = &[#(#schemas),*];
                FIELDS
            }
        }
    })
}

fn is_json_primitive_type(kind: &Type) -> bool {
    let Type::Path(TypePath {
        qself: None, path, ..
    }) = kind
    else {
        return false;
    };
    path.segments.last().is_some_and(|segment| {
        matches!(
            segment.ident.to_string().as_str(),
            "bool"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "f32"
                | "f64"
        )
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldSource {
    Path,
    Query,
    Body,
}

fn direct_generic_type<'a>(kind: &'a Type, expected: &str) -> Option<&'a Type> {
    let Type::Path(TypePath {
        qself: None, path, ..
    }) = kind
    else {
        return None;
    };
    let segment = path.segments.last()?;
    if segment.ident != expected {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    if arguments.args.len() != 1 {
        return None;
    }
    match arguments.args.first()? {
        GenericArgument::Type(kind) => Some(kind),
        _ => None,
    }
}

fn validate_name(value: &str, tokens: &impl quote::ToTokens) -> syn::Result<()> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            tokens,
            "RPC field names must contain 1-128 ASCII letters, digits, '.', '_', or '-'",
        ))
    }
}
