use crate::StrategyAttr;
use proc_macro::TokenStream;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Fields, Meta, parse_macro_input};

pub fn debug(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    let ident = &input.ident;
    let Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(&input, "StrategyDebug only supports structs")
            .into_compile_error()
            .into();
    };
    let Fields::Named(named) = &data.fields else {
        return syn::Error::new_spanned(&input, "StrategyDebug requires named fields")
            .into_compile_error()
            .into();
    };
    let mut fields = Vec::new();
    for field in &named.named {
        let Some(field_ident) = &field.ident else {
            continue;
        };
        let name = field_ident.to_string().trim_start_matches("r#").to_owned();
        let strategy = match strategy(&field.attrs) {
            Ok(value) => value,
            Err(error) => return error.into_compile_error().into(),
        };
        if strategy.ignore.is_some() {
            fields.push(quote! { .field(#name, &"...") });
        } else if strategy.mask.is_some() {
            fields.push(quote! { .field(#name, &fusen_common::string::mask_str(&format!("{:?}", self.#field_ident))) });
        } else if let Some(limit) = strategy.limit {
            let Ok(limit) = limit.parse::<usize>() else {
                return syn::Error::new_spanned(
                    field,
                    "strategy limit must be a non-negative integer",
                )
                .into_compile_error()
                .into();
            };
            fields.push(quote! { .field(#name, &fusen_common::string::limit_str(&format!("{:?}", self.#field_ident), #limit)) });
        } else {
            fields.push(quote! { .field(#name, &self.#field_ident) });
        }
    }
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    quote! {
        impl #impl_generics std::fmt::Debug for #ident #type_generics #where_clause {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.debug_struct(stringify!(#ident)) #(#fields)* .finish()
            }
        }
    }
    .into()
}

fn strategy(attrs: &[Attribute]) -> Result<StrategyAttr, syn::Error> {
    for attr in attrs {
        if let Meta::List(list) = &attr.meta
            && list.path.is_ident("strategy")
        {
            return StrategyAttr::from_attr(list.tokens.clone().into());
        }
    }
    Ok(StrategyAttr::default())
}
