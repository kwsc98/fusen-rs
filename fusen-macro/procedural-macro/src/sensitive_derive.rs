//! Expansion for the `SensitiveFields` derive.

use crate::sensitive::{SensitiveOverride, parse_sensitive_attrs, parse_sensitive_container_attrs};
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, Field, Fields, Ident, Lit, LitStr, Meta, Token,
    Type,
};

pub(crate) fn expand(input: DeriveInput, contract: &TokenStream) -> syn::Result<TokenStream> {
    let sensitive = parse_sensitive_container_attrs(&input.attrs)?;
    if sensitive.value.is_some() {
        reject_nested_overrides(&input.data)?;
    }

    let type_parameters = input
        .generics
        .type_params()
        .map(|parameter| parameter.ident.to_string())
        .collect::<BTreeSet<_>>();
    let mut required_field_bounds = Vec::new();

    let shape = if let Some(value) = &sensitive.value {
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
    if let Some(bounds) = sensitive.bounds {
        generics.make_where_clause().predicates.extend(bounds);
    } else {
        let mut seen_bounds = BTreeSet::new();
        for ty in required_field_bounds {
            if seen_bounds.insert(ty.to_token_stream().to_string()) {
                generics
                    .make_where_clause()
                    .predicates
                    .push(syn::parse2(quote!(#ty: #contract::SensitiveFields))?);
            }
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
            reject_container_representation_changes(&serde)?;
            if let Some(transparent_span) = serde.transparent {
                transparent_shape(
                    &data.fields,
                    transparent_span,
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
                        "tuple and unit structs require a type-level sensitivity kind, `opaque`, or a `#[serde(transparent)]` representation",
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
    rename_all: Directional<Option<RenameRule>>,
    contract: &TokenStream,
    type_ident: &Ident,
    type_parameters: &BTreeSet<String>,
    required_field_bounds: &mut Vec<Type>,
) -> syn::Result<TokenStream> {
    let mut serialized_names = BTreeMap::new();
    let mut deserialized_names = BTreeMap::new();
    let mut serialize_fields = Vec::new();
    let mut deserialize_fields = Vec::new();

    for field in &fields.named {
        let sensitivity = parse_sensitive_attrs(&field.attrs)?;
        let serde = parse_field_serde(field)?;
        if serde.flatten.is_some() {
            return Err(syn::Error::new_spanned(
                field,
                "a flattened serde field cannot be mapped safely; classify the complete type with a type-level sensitivity kind or `opaque`",
            ));
        }
        reject_unclassified_field_customization(field, &serde, sensitivity.as_ref())?;
        if serde.skip.serialize && serde.skip.deserialize {
            continue;
        }

        let field_ident = field
            .ident
            .as_ref()
            .expect("named fields always have identifiers");
        let rust_name = field_ident.to_string();
        let rust_name = rust_name.trim_start_matches("r#");
        let serialize_name = serde.rename.serialize.clone().unwrap_or_else(|| {
            SerdeName::generated(
                rename_all
                    .serialize
                    .map_or_else(|| rust_name.to_owned(), |rule| rule.apply(rust_name)),
                field_ident.span(),
            )
        });
        let deserialize_name = serde.rename.deserialize.clone().unwrap_or_else(|| {
            SerdeName::generated(
                rename_all
                    .deserialize
                    .map_or_else(|| rust_name.to_owned(), |rule| rule.apply(rust_name)),
                field_ident.span(),
            )
        });

        let resolver = match sensitivity.as_ref() {
            Some(value) => override_resolver(value, contract),
            None => {
                record_field_bounds(
                    &field.ty,
                    type_ident,
                    type_parameters,
                    required_field_bounds,
                );
                let ty = &field.ty;
                quote!(<#ty as #contract::SensitiveFields>::sensitive_shape)
            }
        };

        if !serde.skip.serialize {
            insert_directional_name(&mut serialized_names, &serialize_name, "serialized")?;
            serialize_fields.push(sensitive_field(&serialize_name, &resolver, contract));
        }

        if !serde.skip.deserialize {
            let mut names = BTreeMap::new();
            names.insert(deserialize_name.value.clone(), deserialize_name);
            for alias in serde.aliases {
                names.entry(alias.value.clone()).or_insert(alias);
            }
            for name in names.into_values() {
                insert_directional_name(&mut deserialized_names, &name, "deserialized")?;
                deserialize_fields.push(sensitive_field(&name, &resolver, contract));
            }
        }
    }

    Ok(quote! {
        #contract::SensitiveShape::Fields {
            serialize: &[#(#serialize_fields),*],
            deserialize: &[#(#deserialize_fields),*],
        }
    })
}

fn transparent_shape(
    fields: &Fields,
    transparent_span: Span,
    contract: &TokenStream,
    type_ident: &Ident,
    type_parameters: &BTreeSet<String>,
    required_field_bounds: &mut Vec<Type>,
) -> syn::Result<TokenStream> {
    let mut parsed = Vec::with_capacity(fields.len());
    for field in fields {
        let sensitivity = parse_sensitive_attrs(&field.attrs)?;
        let serde = parse_field_serde(field)?;
        if serde.flatten.is_some() {
            return Err(syn::Error::new_spanned(
                field,
                "a flattened serde field cannot be mapped safely; classify the complete type with a type-level sensitivity kind or `opaque`",
            ));
        }
        parsed.push((field, sensitivity, serde));
    }

    let serialize = parsed
        .iter()
        .enumerate()
        .filter(|(_, (field, _, serde))| !is_phantom_data(&field.ty) && !serde.skip.serialize)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let deserialize = parsed
        .iter()
        .enumerate()
        .filter(|(_, (field, _, serde))| {
            !is_phantom_data(&field.ty) && !serde.skip.deserialize && !serde.default
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    let serialize = exactly_one_transparent_field(
        &serialize,
        transparent_span,
        "serialization",
        "not skipped",
    )?;
    let deserialize = exactly_one_transparent_field(
        &deserialize,
        transparent_span,
        "deserialization",
        "neither skipped nor defaulted",
    )?;
    if serialize != deserialize {
        return Err(syn::Error::new(
            transparent_span,
            "`#[serde(transparent)]` must use the same field for serialization and deserialization when deriving `SensitiveFields`",
        ));
    }

    let (field, sensitivity, serde) = &parsed[serialize];
    reject_unclassified_field_customization(field, serde, sensitivity.as_ref())?;
    Ok(match sensitivity.as_ref() {
        Some(value) => override_shape(value, contract),
        None => {
            record_field_bounds(
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

fn exactly_one_transparent_field(
    candidates: &[usize],
    span: Span,
    direction: &str,
    requirement: &str,
) -> syn::Result<usize> {
    match candidates {
        [field] => Ok(*field),
        [] => Err(syn::Error::new(
            span,
            format!(
                "`#[serde(transparent)]` requires one field that is {requirement} for {direction}"
            ),
        )),
        _ => Err(syn::Error::new(
            span,
            format!(
                "`#[serde(transparent)]` permits at most one field for {direction}; skip or default the remaining fields according to serde rules"
            ),
        )),
    }
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

fn record_field_bounds(
    ty: &Type,
    type_ident: &Ident,
    type_parameters: &BTreeSet<String>,
    required_field_bounds: &mut Vec<Type>,
) {
    if is_direct_recursive_type(ty, type_ident) {
        return;
    }
    let usage = type_usage(ty, type_ident, type_parameters);
    if !usage.uses_parameter {
        return;
    }
    if !usage.is_recursive {
        required_field_bounds.push(ty.clone());
        return;
    }

    let mut children = DirectTypeChildren::default();
    syn::visit::visit_type(&mut children, ty);
    for child in children.types {
        record_field_bounds(child, type_ident, type_parameters, required_field_bounds);
    }
}

fn is_direct_recursive_type(ty: &Type, type_ident: &Ident) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    path.qself.is_none() && is_recursive_path(&path.path, type_ident)
}

fn is_recursive_path(path: &syn::Path, type_ident: &Ident) -> bool {
    if path.leading_colon.is_some() {
        return false;
    }
    match path.segments.len() {
        1 => path
            .segments
            .first()
            .is_some_and(|segment| segment.ident == "Self" || segment.ident == *type_ident),
        2 => {
            let mut segments = path.segments.iter();
            let first = segments.next().expect("the path contains two segments");
            let last = segments.next().expect("the path contains two segments");
            first.ident == "self" && last.ident == *type_ident
        }
        _ => false,
    }
}

#[derive(Default)]
struct TypeUsage {
    uses_parameter: bool,
    is_recursive: bool,
}

fn type_usage(ty: &Type, type_ident: &Ident, type_parameters: &BTreeSet<String>) -> TypeUsage {
    struct Usage<'a> {
        type_ident: &'a Ident,
        type_parameters: &'a BTreeSet<String>,
        result: TypeUsage,
    }

    impl<'ast> Visit<'ast> for Usage<'_> {
        fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
            for segment in &path.path.segments {
                if self.type_parameters.contains(&segment.ident.to_string()) {
                    self.result.uses_parameter = true;
                }
            }
            if path.qself.is_none() {
                self.result.is_recursive |= is_recursive_path(&path.path, self.type_ident);
            }
            syn::visit::visit_type_path(self, path);
        }
    }

    let mut usage = Usage {
        type_ident,
        type_parameters,
        result: TypeUsage::default(),
    };
    usage.visit_type(ty);
    usage.result
}

#[derive(Default)]
struct DirectTypeChildren<'ast> {
    types: Vec<&'ast Type>,
}

impl<'ast> Visit<'ast> for DirectTypeChildren<'ast> {
    fn visit_type(&mut self, ty: &'ast Type) {
        self.types.push(ty);
    }
}

#[derive(Clone, Default)]
struct Directional<T> {
    serialize: T,
    deserialize: T,
}

#[derive(Clone)]
struct SerdeName {
    value: String,
    span: Span,
}

impl SerdeName {
    fn explicit(value: LitStr) -> Self {
        Self {
            value: value.value(),
            span: value.span(),
        }
    }

    fn generated(value: String, span: Span) -> Self {
        Self { value, span }
    }

    fn literal(&self) -> LitStr {
        LitStr::new(&self.value, self.span)
    }
}

#[derive(Clone)]
struct RepresentationChange {
    attribute: &'static str,
    span: Span,
}

#[derive(Default)]
struct ContainerSerde {
    transparent: Option<Span>,
    rename_all: Directional<Option<RenameRule>>,
    representation_change: Directional<Option<RepresentationChange>>,
}

fn parse_container_serde(attributes: &[Attribute]) -> syn::Result<ContainerSerde> {
    let mut serde = ContainerSerde::default();
    for meta in serde_meta(attributes)? {
        match meta {
            Meta::Path(path) if path.is_ident("transparent") => {
                if serde.transparent.replace(path.span()).is_some() {
                    return Err(syn::Error::new_spanned(
                        path,
                        "duplicate serde `transparent` declaration",
                    ));
                }
            }
            Meta::NameValue(meta) if meta.path.is_ident("rename_all") => {
                let value = string_literal(meta.value, "rename_all")?;
                let rule = RenameRule::parse_literal(&value)?;
                set_rename_rule(
                    &mut serde.rename_all.serialize,
                    rule,
                    &meta.path,
                    "serialization",
                )?;
                set_rename_rule(
                    &mut serde.rename_all.deserialize,
                    rule,
                    &meta.path,
                    "deserialization",
                )?;
            }
            Meta::List(meta) if meta.path.is_ident("rename_all") => {
                let rules = directional_rename_rules(&meta, "rename_all")?;
                if let Some(rule) = rules.serialize {
                    set_rename_rule(
                        &mut serde.rename_all.serialize,
                        rule,
                        &meta.path,
                        "serialization",
                    )?;
                }
                if let Some(rule) = rules.deserialize {
                    set_rename_rule(
                        &mut serde.rename_all.deserialize,
                        rule,
                        &meta.path,
                        "deserialization",
                    )?;
                }
            }
            Meta::NameValue(meta) if meta.path.is_ident("into") => {
                set_representation_change(
                    &mut serde.representation_change.serialize,
                    "into",
                    meta.path.span(),
                );
            }
            Meta::NameValue(meta)
                if meta.path.is_ident("from") || meta.path.is_ident("try_from") =>
            {
                let attribute = if meta.path.is_ident("from") {
                    "from"
                } else {
                    "try_from"
                };
                set_representation_change(
                    &mut serde.representation_change.deserialize,
                    attribute,
                    meta.path.span(),
                );
            }
            Meta::NameValue(meta) if meta.path.is_ident("remote") => {
                set_representation_change(
                    &mut serde.representation_change.serialize,
                    "remote",
                    meta.path.span(),
                );
                set_representation_change(
                    &mut serde.representation_change.deserialize,
                    "remote",
                    meta.path.span(),
                );
            }
            Meta::NameValue(meta) if meta.path.is_ident("tag") || meta.path.is_ident("content") => {
                let attribute = if meta.path.is_ident("tag") {
                    "tag"
                } else {
                    "content"
                };
                set_representation_change(
                    &mut serde.representation_change.serialize,
                    attribute,
                    meta.path.span(),
                );
                set_representation_change(
                    &mut serde.representation_change.deserialize,
                    attribute,
                    meta.path.span(),
                );
            }
            Meta::Path(path) if path.is_ident("untagged") => {
                return Err(syn::Error::new(
                    path.span(),
                    "`#[serde(untagged)]` changes the complete representation; classify the type with `#[sensitive(kind = \"...\")]` or `#[sensitive(opaque)]`",
                ));
            }
            _ => {}
        }
    }
    Ok(serde)
}

fn set_representation_change(
    slot: &mut Option<RepresentationChange>,
    attribute: &'static str,
    span: Span,
) {
    if slot.is_none() {
        *slot = Some(RepresentationChange { attribute, span });
    }
}

fn reject_container_representation_changes(serde: &ContainerSerde) -> syn::Result<()> {
    for (direction, change) in [
        ("serialized", serde.representation_change.serialize.as_ref()),
        (
            "deserialized",
            serde.representation_change.deserialize.as_ref(),
        ),
    ] {
        if let Some(change) = change {
            return Err(syn::Error::new(
                change.span,
                format!(
                    "`#[serde({} = \"...\")]` changes the complete {direction} representation; classify the type with `#[sensitive(kind = \"...\")]` or `#[sensitive(opaque)]`",
                    change.attribute
                ),
            ));
        }
    }
    Ok(())
}

#[derive(Default)]
struct FieldSerde {
    rename: Directional<Option<SerdeName>>,
    aliases: Vec<SerdeName>,
    skip: Directional<bool>,
    default: bool,
    flatten: Option<Span>,
    custom: Directional<Option<RepresentationChange>>,
}

fn parse_field_serde(field: &Field) -> syn::Result<FieldSerde> {
    let mut serde = FieldSerde::default();
    for meta in serde_meta(&field.attrs)? {
        match meta {
            Meta::Path(path) if path.is_ident("skip") => {
                serde.skip.serialize = true;
                serde.skip.deserialize = true;
            }
            Meta::Path(path) if path.is_ident("skip_serializing") => {
                serde.skip.serialize = true;
            }
            Meta::Path(path) if path.is_ident("skip_deserializing") => {
                serde.skip.deserialize = true;
            }
            Meta::Path(path) if path.is_ident("default") => serde.default = true,
            Meta::NameValue(meta) if meta.path.is_ident("default") => serde.default = true,
            Meta::Path(path) if path.is_ident("flatten") => {
                serde.flatten = Some(path.span());
            }
            Meta::NameValue(meta) if meta.path.is_ident("rename") => {
                let value = SerdeName::explicit(string_literal(meta.value, "rename")?);
                set_serde_name(
                    &mut serde.rename.serialize,
                    value.clone(),
                    &meta.path,
                    "serialization",
                )?;
                set_serde_name(
                    &mut serde.rename.deserialize,
                    value,
                    &meta.path,
                    "deserialization",
                )?;
            }
            Meta::List(meta) if meta.path.is_ident("rename") => {
                let names = directional_names(&meta, "rename")?;
                if let Some(name) = names.serialize {
                    set_serde_name(
                        &mut serde.rename.serialize,
                        name,
                        &meta.path,
                        "serialization",
                    )?;
                }
                if let Some(name) = names.deserialize {
                    set_serde_name(
                        &mut serde.rename.deserialize,
                        name,
                        &meta.path,
                        "deserialization",
                    )?;
                }
            }
            Meta::NameValue(meta) if meta.path.is_ident("alias") => {
                serde
                    .aliases
                    .push(SerdeName::explicit(string_literal(meta.value, "alias")?));
            }
            Meta::NameValue(meta) if meta.path.is_ident("serialize_with") => {
                set_customization(
                    &mut serde.custom.serialize,
                    "serialize_with",
                    meta.path.span(),
                    "serialization",
                )?;
            }
            Meta::NameValue(meta) if meta.path.is_ident("deserialize_with") => {
                set_customization(
                    &mut serde.custom.deserialize,
                    "deserialize_with",
                    meta.path.span(),
                    "deserialization",
                )?;
            }
            Meta::NameValue(meta) if meta.path.is_ident("with") => {
                set_customization(
                    &mut serde.custom.serialize,
                    "with",
                    meta.path.span(),
                    "serialization",
                )?;
                set_customization(
                    &mut serde.custom.deserialize,
                    "with",
                    meta.path.span(),
                    "deserialization",
                )?;
            }
            Meta::NameValue(meta) if meta.path.is_ident("getter") => {
                set_customization(
                    &mut serde.custom.serialize,
                    "getter",
                    meta.path.span(),
                    "serialization",
                )?;
            }
            _ => {}
        }
    }
    Ok(serde)
}

fn set_customization(
    slot: &mut Option<RepresentationChange>,
    attribute: &'static str,
    span: Span,
    direction: &str,
) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new(
            span,
            format!("duplicate serde {direction} customization"),
        ));
    }
    *slot = Some(RepresentationChange { attribute, span });
    Ok(())
}

fn reject_unclassified_field_customization(
    field: &Field,
    serde: &FieldSerde,
    sensitivity: Option<&SensitiveOverride>,
) -> syn::Result<()> {
    if sensitivity.is_some() {
        return Ok(());
    }
    if !serde.skip.serialize
        && let Some(custom) = &serde.custom.serialize
    {
        return Err(syn::Error::new(
            custom.span,
            format!(
                "a field using serde `{}` requires `#[sensitive(kind = \"...\")]` or `#[sensitive(opaque)]` because its serialized representation may differ from the Rust field type",
                custom.attribute
            ),
        ));
    }
    if !serde.skip.deserialize
        && let Some(custom) = &serde.custom.deserialize
    {
        return Err(syn::Error::new(
            custom.span,
            format!(
                "a field using serde `{}` requires `#[sensitive(kind = \"...\")]` or `#[sensitive(opaque)]` because its deserialized representation may differ from the Rust field type",
                custom.attribute
            ),
        ));
    }
    let _ = field;
    Ok(())
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

fn directional_names(
    meta: &syn::MetaList,
    field: &str,
) -> syn::Result<Directional<Option<SerdeName>>> {
    let nested = meta.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
    let mut result = Directional::default();
    for nested in nested {
        let Meta::NameValue(nested) = nested else {
            return Err(syn::Error::new_spanned(
                nested,
                format!(
                    "serde `{field}` directions must use `serialize = \"...\"` or `deserialize = \"...\"`"
                ),
            ));
        };
        if nested.path.is_ident("serialize") {
            let value = SerdeName::explicit(string_literal(nested.value, field)?);
            set_serde_name(&mut result.serialize, value, &nested.path, "serialization")?;
        } else if nested.path.is_ident("deserialize") {
            let value = SerdeName::explicit(string_literal(nested.value, field)?);
            set_serde_name(
                &mut result.deserialize,
                value,
                &nested.path,
                "deserialization",
            )?;
        } else {
            return Err(syn::Error::new_spanned(
                nested.path,
                format!("unknown serde `{field}` direction; expected `serialize` or `deserialize`"),
            ));
        }
    }
    if result.serialize.is_none() && result.deserialize.is_none() {
        return Err(syn::Error::new_spanned(
            meta,
            format!("serde `{field}` requires `serialize = \"...\"` or `deserialize = \"...\"`"),
        ));
    }
    Ok(result)
}

fn directional_rename_rules(
    meta: &syn::MetaList,
    field: &str,
) -> syn::Result<Directional<Option<RenameRule>>> {
    let names = directional_names(meta, field)?;
    Ok(Directional {
        serialize: names
            .serialize
            .as_ref()
            .map(RenameRule::parse)
            .transpose()?,
        deserialize: names
            .deserialize
            .as_ref()
            .map(RenameRule::parse)
            .transpose()?,
    })
}

fn set_serde_name(
    slot: &mut Option<SerdeName>,
    value: SerdeName,
    span: &impl ToTokens,
    direction: &str,
) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new_spanned(
            span,
            format!("duplicate serde {direction} rename"),
        ));
    }
    *slot = Some(value);
    Ok(())
}

fn set_rename_rule(
    slot: &mut Option<RenameRule>,
    value: RenameRule,
    span: &impl ToTokens,
    direction: &str,
) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new_spanned(
            span,
            format!("duplicate serde {direction} rename rule"),
        ));
    }
    *slot = Some(value);
    Ok(())
}

fn string_literal(value: Expr, field: &str) -> syn::Result<LitStr> {
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
    Ok(value)
}

fn insert_directional_name(
    names: &mut BTreeMap<String, Span>,
    name: &SerdeName,
    direction: &str,
) -> syn::Result<()> {
    if let Some(first_span) = names.get(&name.value).copied() {
        let mut error = syn::Error::new(
            name.span,
            format!(
                "duplicate {direction} field name `{}` after applying serde renames and aliases",
                name.value
            ),
        );
        error.combine(syn::Error::new(
            first_span,
            "first field name declared here",
        ));
        return Err(error);
    }
    names.insert(name.value.clone(), name.span);
    Ok(())
}

fn sensitive_field(
    name: &SerdeName,
    resolver: &TokenStream,
    contract: &TokenStream,
) -> TokenStream {
    let name = name.literal();
    quote! {
        const { #contract::SensitiveField::new(#name, #resolver) }
    }
}

fn is_phantom_data(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "PhantomData")
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
    fn parse(value: &SerdeName) -> syn::Result<Self> {
        match value.value.as_str() {
            "lowercase" => Ok(Self::Lower),
            "UPPERCASE" => Ok(Self::Upper),
            "PascalCase" => Ok(Self::Pascal),
            "camelCase" => Ok(Self::Camel),
            "snake_case" => Ok(Self::Snake),
            "SCREAMING_SNAKE_CASE" => Ok(Self::ScreamingSnake),
            "kebab-case" => Ok(Self::Kebab),
            "SCREAMING-KEBAB-CASE" => Ok(Self::ScreamingKebab),
            _ => Err(syn::Error::new(
                value.span,
                "unsupported serde rename rule; expected `lowercase`, `UPPERCASE`, `PascalCase`, `camelCase`, `snake_case`, `SCREAMING_SNAKE_CASE`, `kebab-case`, or `SCREAMING-KEBAB-CASE`",
            )),
        }
    }

    fn parse_literal(value: &LitStr) -> syn::Result<Self> {
        Self::parse(&SerdeName::explicit(value.clone()))
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
    use quote::quote;

    #[test]
    fn applies_serde_field_rename_rules() {
        assert_eq!(RenameRule::Lower.apply("mixed_Name"), "mixed_Name");
        assert_eq!(RenameRule::Camel.apply("user_name"), "userName");
        assert_eq!(RenameRule::Pascal.apply("user_name"), "UserName");
        assert_eq!(RenameRule::Kebab.apply("user_name"), "user-name");
        assert_eq!(RenameRule::ScreamingSnake.apply("user_name"), "USER_NAME");
    }

    #[test]
    fn parses_directional_field_names_and_aliases() {
        let input: DeriveInput = syn::parse2(quote! {
            struct Request {
                #[serde(
                    rename(serialize = "user-id", deserialize = "user_id"),
                    alias = "legacy_id",
                    skip_serializing
                )]
                user_id: String,
            }
        })
        .unwrap();
        let Data::Struct(data) = input.data else {
            panic!("test input should be a struct");
        };
        let field = data.fields.iter().next().unwrap();
        let serde = parse_field_serde(field).unwrap();
        assert_eq!(serde.rename.serialize.unwrap().value, "user-id");
        assert_eq!(serde.rename.deserialize.unwrap().value, "user_id");
        assert_eq!(serde.aliases[0].value, "legacy_id");
        assert!(serde.skip.serialize);
        assert!(!serde.skip.deserialize);
    }

    #[test]
    fn recursive_generic_bounds_descend_into_non_recursive_subtrees() {
        let ty: Type = syn::parse2(quote!((Box<Node<T>>, T, Wrapper<U>))).unwrap();
        let type_ident = Ident::new("Node", Span::call_site());
        let type_parameters = ["T".to_owned(), "U".to_owned()].into_iter().collect();
        let mut bounds = Vec::new();
        record_field_bounds(&ty, &type_ident, &type_parameters, &mut bounds);
        let bounds = bounds
            .iter()
            .map(|bound| bound.to_token_stream().to_string())
            .collect::<Vec<_>>();
        assert_eq!(bounds, ["T", "Wrapper < U >"]);
    }

    #[test]
    fn recursive_generic_arguments_do_not_create_spurious_bounds() {
        let ty: Type = syn::parse2(quote!(Option<Box<Node<T>>>)).unwrap();
        let type_ident = Ident::new("Node", Span::call_site());
        let type_parameters = ["T".to_owned()].into_iter().collect();
        let mut bounds = Vec::new();
        record_field_bounds(&ty, &type_ident, &type_parameters, &mut bounds);
        assert!(bounds.is_empty());
    }

    #[test]
    fn recognizes_only_unambiguous_recursive_paths() {
        let type_ident = Ident::new("Node", Span::call_site());
        for tokens in [quote!(Node<T>), quote!(Self), quote!(self::Node<T>)] {
            let ty: Type = syn::parse2(tokens).unwrap();
            assert!(is_direct_recursive_type(&ty, &type_ident));
        }

        for tokens in [
            quote!(Self::Value),
            quote!(external::Node<T>),
            quote!(crate::dto::Node<T>),
            quote!(super::super::Node<T>),
        ] {
            let ty: Type = syn::parse2(tokens).unwrap();
            assert!(!is_direct_recursive_type(&ty, &type_ident));
        }
    }
}
