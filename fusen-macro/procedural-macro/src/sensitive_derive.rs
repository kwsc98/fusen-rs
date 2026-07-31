//! Expansion for the `SensitiveFields` derive.

use crate::sensitive::{SensitiveOverride, parse_sensitive_attrs};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::collections::BTreeSet;
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, Field, Fields, Ident, Lit, LitStr, Meta, Token,
    Type,
};

pub(crate) fn expand(input: DeriveInput, contract: &TokenStream) -> syn::Result<TokenStream> {
    let type_override = parse_sensitive_attrs(&input.attrs)?;
    if type_override.is_some() {
        reject_nested_overrides(&input.data)?;
    }

    let type_parameters = input
        .generics
        .type_params()
        .map(|parameter| parameter.ident.to_string())
        .collect::<BTreeSet<_>>();
    let mut required_field_bounds = Vec::new();

    let shape = if let Some(value) = &type_override {
        override_shape(value, contract)
    } else {
        derive_data_shape(
            &input.data,
            &input.attrs,
            contract,
            &input.ident,
            &type_parameters,
            &mut required_field_bounds,
        )?
    };

    let ident = &input.ident;
    let mut generics = input.generics.clone();
    let mut seen_bounds = BTreeSet::new();
    for ty in required_field_bounds {
        if seen_bounds.insert(ty.to_token_stream().to_string()) {
            generics
                .make_where_clause()
                .predicates
                .push(syn::parse2(quote!(#ty: #contract::SensitiveFields))?);
        }
    }
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #contract::SensitiveFields for #ident #type_generics #where_clause {
            fn sensitive_shape() -> #contract::SensitiveShape {
                #shape
            }
        }
    })
}

fn derive_data_shape(
    data: &Data,
    attributes: &[Attribute],
    contract: &TokenStream,
    type_ident: &Ident,
    type_parameters: &BTreeSet<String>,
    required_field_bounds: &mut Vec<Type>,
) -> syn::Result<TokenStream> {
    match data {
        Data::Struct(data) => {
            let serde = parse_container_serde(attributes)?;
            if serde.custom_serializer {
                return Err(syn::Error::new_spanned(
                    data.struct_token,
                    "a struct with a custom serde serializer requires a type-level `#[sensitive(kind = \"...\")]` or `#[sensitive(opaque)]`",
                ));
            }
            if serde.transparent {
                transparent_shape(
                    &data.fields,
                    contract,
                    type_ident,
                    type_parameters,
                    required_field_bounds,
                )
            } else {
                match &data.fields {
                    Fields::Named(fields) => named_fields_shape(
                        fields,
                        serde.rename_all,
                        contract,
                        type_ident,
                        type_parameters,
                        required_field_bounds,
                    ),
                    Fields::Unnamed(_) | Fields::Unit => Err(syn::Error::new_spanned(
                        &data.fields,
                        "tuple and unit structs require a type-level sensitivity kind, `opaque`, or a single-field `#[serde(transparent)]` representation",
                    )),
                }
            }
        }
        Data::Enum(data) => Err(syn::Error::new_spanned(
            data.enum_token,
            "enums require a type-level `#[sensitive(kind = \"...\")]` or `#[sensitive(opaque)]`",
        )),
        Data::Union(data) => Err(syn::Error::new_spanned(
            data.union_token,
            "unions require a type-level `#[sensitive(kind = \"...\")]` or `#[sensitive(opaque)]`",
        )),
    }
}

fn named_fields_shape(
    fields: &syn::FieldsNamed,
    rename_all: Option<RenameRule>,
    contract: &TokenStream,
    type_ident: &Ident,
    type_parameters: &BTreeSet<String>,
    required_field_bounds: &mut Vec<Type>,
) -> syn::Result<TokenStream> {
    let mut serialized_names = BTreeSet::new();
    let mut schema_fields = Vec::new();

    for field in &fields.named {
        let sensitivity = parse_sensitive_attrs(&field.attrs)?;
        let serde = parse_field_serde(field)?;
        if serde.skip {
            continue;
        }
        if serde.flatten {
            return Err(syn::Error::new_spanned(
                field,
                "a flattened serde field cannot be mapped safely; classify the complete type with a type-level sensitivity kind or `opaque`",
            ));
        }
        if serde.custom_serializer && sensitivity.is_none() {
            return Err(syn::Error::new_spanned(
                field,
                "a field with a custom serde serializer requires `#[sensitive(kind = \"...\")]` or `#[sensitive(opaque)]`",
            ));
        }

        let rust_name = field
            .ident
            .as_ref()
            .expect("named fields always have identifiers")
            .to_string();
        let rust_name = rust_name.trim_start_matches("r#");
        let serialized_name = serde.rename.unwrap_or_else(|| {
            rename_all.map_or_else(|| rust_name.to_owned(), |rule| rule.apply(rust_name))
        });
        if !serialized_names.insert(serialized_name.clone()) {
            return Err(syn::Error::new_spanned(
                field,
                format!(
                    "duplicate serialized field name `{serialized_name}` after applying serde renames"
                ),
            ));
        }

        let resolver = match sensitivity {
            Some(value) => override_resolver(&value, contract),
            None => {
                record_field_bound(
                    &field.ty,
                    type_ident,
                    type_parameters,
                    required_field_bounds,
                );
                let ty = &field.ty;
                quote!(<#ty as #contract::SensitiveFields>::sensitive_shape)
            }
        };
        schema_fields.push(quote! {
            const { #contract::SensitiveField::new(#serialized_name, #resolver) }
        });
    }

    Ok(quote! {
        #contract::SensitiveShape::Fields(&[#(#schema_fields),*])
    })
}

fn transparent_shape(
    fields: &Fields,
    contract: &TokenStream,
    type_ident: &Ident,
    type_parameters: &BTreeSet<String>,
    required_field_bounds: &mut Vec<Type>,
) -> syn::Result<TokenStream> {
    if fields.len() != 1 {
        return Err(syn::Error::new_spanned(
            fields,
            "`#[serde(transparent)]` sensitivity derivation requires exactly one field",
        ));
    }
    let field = fields.iter().next().expect("the field count was checked");
    let sensitivity = parse_sensitive_attrs(&field.attrs)?;
    let serde = parse_field_serde(field)?;
    if serde.skip || serde.flatten {
        return Err(syn::Error::new_spanned(
            field,
            "the field of a transparent type must be serialized directly",
        ));
    }
    if serde.custom_serializer && sensitivity.is_none() {
        return Err(syn::Error::new_spanned(
            field,
            "a transparent field with a custom serde serializer requires `#[sensitive(kind = \"...\")]` or `#[sensitive(opaque)]`",
        ));
    }

    Ok(match sensitivity {
        Some(value) => override_shape(&value, contract),
        None => {
            record_field_bound(
                &field.ty,
                type_ident,
                type_parameters,
                required_field_bounds,
            );
            let ty = &field.ty;
            quote!(<#ty as #contract::SensitiveFields>::sensitive_shape())
        }
    })
}

fn reject_nested_overrides(data: &Data) -> syn::Result<()> {
    let fields: Box<dyn Iterator<Item = &Field> + '_> = match data {
        Data::Struct(data) => Box::new(data.fields.iter()),
        Data::Enum(data) => Box::new(
            data.variants
                .iter()
                .flat_map(|variant| variant.fields.iter()),
        ),
        Data::Union(data) => Box::new(data.fields.named.iter()),
    };
    for field in fields {
        if let Some(attribute) = field
            .attrs
            .iter()
            .find(|attribute| crate::sensitive::is_sensitive_attr(attribute))
        {
            return Err(syn::Error::new_spanned(
                attribute,
                "field sensitivity declarations cannot be combined with a type-level sensitivity declaration",
            ));
        }
    }
    if let Data::Enum(data) = data {
        for variant in &data.variants {
            if let Some(attribute) = variant
                .attrs
                .iter()
                .find(|attribute| crate::sensitive::is_sensitive_attr(attribute))
            {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "variant sensitivity declarations are not supported",
                ));
            }
        }
    }
    Ok(())
}

fn override_shape(value: &SensitiveOverride, contract: &TokenStream) -> TokenStream {
    match value {
        SensitiveOverride::Kind(kind) => {
            let kind = kind_expression(kind, contract);
            quote!(#contract::SensitiveShape::Kind(#kind))
        }
        SensitiveOverride::Opaque => quote!(#contract::SensitiveShape::Opaque),
    }
}

fn override_resolver(value: &SensitiveOverride, contract: &TokenStream) -> TokenStream {
    let shape = override_shape(value, contract);
    quote!(|| #shape)
}

fn kind_expression(kind: &LitStr, contract: &TokenStream) -> TokenStream {
    let known = match kind.value().as_str() {
        "public" => Some(quote!(PUBLIC)),
        "credential" => Some(quote!(CREDENTIAL)),
        "token" => Some(quote!(TOKEN)),
        "phone" => Some(quote!(PHONE)),
        "email" => Some(quote!(EMAIL)),
        "identifier" => Some(quote!(IDENTIFIER)),
        "secret" => Some(quote!(SECRET)),
        _ => None,
    };
    known.map_or_else(
        || {
            quote! {
                #contract::SensitivityKind::new(#kind)
                    .expect("macro validated sensitivity kind")
            }
        },
        |constant| quote!(#contract::SensitivityKind::#constant),
    )
}

fn record_field_bound(
    ty: &Type,
    type_ident: &Ident,
    type_parameters: &BTreeSet<String>,
    required_field_bounds: &mut Vec<Type>,
) {
    struct Usage<'a> {
        type_ident: &'a Ident,
        type_parameters: &'a BTreeSet<String>,
        uses_parameter: bool,
        is_recursive: bool,
    }

    impl<'ast> Visit<'ast> for Usage<'_> {
        fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
            for segment in &path.path.segments {
                let name = segment.ident.to_string();
                if self.type_parameters.contains(&name) {
                    self.uses_parameter = true;
                }
            }
            if path.qself.is_none()
                && path.path.leading_colon.is_none()
                && path.path.segments.len() == 1
            {
                let ident = &path.path.segments[0].ident;
                if ident == self.type_ident || ident == "Self" {
                    self.is_recursive = true;
                }
            }
            syn::visit::visit_type_path(self, path);
        }
    }

    let mut usage = Usage {
        type_ident,
        type_parameters,
        uses_parameter: false,
        is_recursive: false,
    };
    usage.visit_type(ty);
    if usage.uses_parameter && !usage.is_recursive {
        required_field_bounds.push(ty.clone());
    }
}

#[derive(Default)]
struct ContainerSerde {
    transparent: bool,
    rename_all: Option<RenameRule>,
    custom_serializer: bool,
}

fn parse_container_serde(attributes: &[Attribute]) -> syn::Result<ContainerSerde> {
    let mut serde = ContainerSerde::default();
    for meta in serde_meta(attributes)? {
        match meta {
            Meta::Path(path) if path.is_ident("transparent") => {
                if serde.transparent {
                    return Err(syn::Error::new_spanned(
                        path,
                        "duplicate serde `transparent` declaration",
                    ));
                }
                serde.transparent = true;
            }
            Meta::NameValue(meta) if meta.path.is_ident("rename_all") => {
                let value = string_value(meta.value, "rename_all")?;
                set_rename_rule(
                    &mut serde.rename_all,
                    RenameRule::parse(&value, &meta.path)?,
                    &meta.path,
                )?;
            }
            Meta::List(meta) if meta.path.is_ident("rename_all") => {
                if let Some(value) = serialization_name(&meta, "rename_all")? {
                    set_rename_rule(
                        &mut serde.rename_all,
                        RenameRule::parse(&value, &meta.path)?,
                        &meta.path,
                    )?;
                }
            }
            Meta::NameValue(meta) if meta.path.is_ident("into") || meta.path.is_ident("remote") => {
                serde.custom_serializer = true;
            }
            _ => {}
        }
    }
    Ok(serde)
}

#[derive(Default)]
struct FieldSerde {
    rename: Option<String>,
    skip: bool,
    flatten: bool,
    custom_serializer: bool,
}

fn parse_field_serde(field: &Field) -> syn::Result<FieldSerde> {
    let mut serde = FieldSerde::default();
    for meta in serde_meta(&field.attrs)? {
        match meta {
            Meta::Path(path) if path.is_ident("skip") || path.is_ident("skip_serializing") => {
                serde.skip = true;
            }
            Meta::Path(path) if path.is_ident("flatten") => serde.flatten = true,
            Meta::NameValue(meta)
                if meta.path.is_ident("serialize_with")
                    || meta.path.is_ident("with")
                    || meta.path.is_ident("getter") =>
            {
                serde.custom_serializer = true;
            }
            Meta::NameValue(meta) if meta.path.is_ident("rename") => {
                set_serde_name(
                    &mut serde.rename,
                    string_value(meta.value, "rename")?,
                    &meta.path,
                )?;
            }
            Meta::List(meta) if meta.path.is_ident("rename") => {
                if let Some(name) = serialization_name(&meta, "rename")? {
                    set_serde_name(&mut serde.rename, name, &meta.path)?;
                }
            }
            _ => {}
        }
    }
    Ok(serde)
}

fn serde_meta(attributes: &[Attribute]) -> syn::Result<Vec<Meta>> {
    let mut result = Vec::new();
    for attribute in attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("serde"))
    {
        let Meta::List(list) = &attribute.meta else {
            return Err(syn::Error::new_spanned(
                attribute,
                "`serde` must use `#[serde(...)]` syntax",
            ));
        };
        result.extend(list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?);
    }
    Ok(result)
}

fn serialization_name(meta: &syn::MetaList, field: &str) -> syn::Result<Option<String>> {
    let nested = meta.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
    let mut value = None;
    for meta in nested {
        if let Meta::NameValue(meta) = meta
            && meta.path.is_ident("serialize")
        {
            set_serde_name(&mut value, string_value(meta.value, field)?, &meta.path)?;
        }
    }
    Ok(value)
}

fn set_serde_name(
    slot: &mut Option<String>,
    value: String,
    span: &impl quote::ToTokens,
) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new_spanned(
            span,
            "duplicate serde serialization rename",
        ));
    }
    *slot = Some(value);
    Ok(())
}

fn set_rename_rule(
    slot: &mut Option<RenameRule>,
    value: RenameRule,
    span: &impl quote::ToTokens,
) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new_spanned(
            span,
            "duplicate serde serialization rename rule",
        ));
    }
    *slot = Some(value);
    Ok(())
}

fn string_value(value: Expr, field: &str) -> syn::Result<String> {
    let Expr::Lit(ExprLit {
        lit: Lit::Str(value),
        ..
    }) = value
    else {
        return Err(syn::Error::new_spanned(
            value,
            format!("serde `{field}` must be a string literal"),
        ));
    };
    Ok(value.value())
}

#[derive(Clone, Copy)]
enum RenameRule {
    Lower,
    Upper,
    Pascal,
    Camel,
    Snake,
    ScreamingSnake,
    Kebab,
    ScreamingKebab,
}

impl RenameRule {
    fn parse(value: &str, span: &impl quote::ToTokens) -> syn::Result<Self> {
        match value {
            "lowercase" => Ok(Self::Lower),
            "UPPERCASE" => Ok(Self::Upper),
            "PascalCase" => Ok(Self::Pascal),
            "camelCase" => Ok(Self::Camel),
            "snake_case" => Ok(Self::Snake),
            "SCREAMING_SNAKE_CASE" => Ok(Self::ScreamingSnake),
            "kebab-case" => Ok(Self::Kebab),
            "SCREAMING-KEBAB-CASE" => Ok(Self::ScreamingKebab),
            _ => Err(syn::Error::new_spanned(
                span,
                "unsupported serde rename rule",
            )),
        }
    }

    fn apply(self, name: &str) -> String {
        match self {
            Self::Lower => name.to_owned(),
            Self::Upper => name.to_ascii_uppercase(),
            Self::Pascal => pascal_case(name),
            Self::Camel => {
                let pascal = pascal_case(name);
                let mut chars = pascal.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_ascii_lowercase().to_string() + chars.as_str()
                })
            }
            Self::Snake => name.to_owned(),
            Self::ScreamingSnake => name.to_ascii_uppercase(),
            Self::Kebab => name.replace('_', "-"),
            Self::ScreamingKebab => name.to_ascii_uppercase().replace('_', "-"),
        }
    }
}

fn pascal_case(name: &str) -> String {
    let mut pascal = String::new();
    let mut capitalize = true;
    for character in name.chars() {
        if character == '_' {
            capitalize = true;
        } else if capitalize {
            pascal.push(character.to_ascii_uppercase());
            capitalize = false;
        } else {
            pascal.push(character);
        }
    }
    pascal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_serde_field_rename_rules() {
        assert_eq!(RenameRule::Lower.apply("mixed_Name"), "mixed_Name");
        assert_eq!(RenameRule::Camel.apply("user_name"), "userName");
        assert_eq!(RenameRule::Pascal.apply("user_name"), "UserName");
        assert_eq!(RenameRule::Kebab.apply("user_name"), "user-name");
        assert_eq!(RenameRule::ScreamingSnake.apply("user_name"), "USER_NAME");
    }
}
