#[macro_export]
macro_rules! fusen_attr {
    ($name:ident, $($field:ident),*) => {
        #[derive(Default)]
        struct $name { $( $field: Option<String> ),* }
        impl $name {
            fn from_attr(args: TokenStream) -> Result<Self, syn::Error> {
                use syn::parse::Parser;
                let args = syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated.parse2(args.into())?;
                let mut value = Self::default();
                for arg in args {
                    match arg {
                        syn::Meta::NameValue(name_value) => {
                            use quote::ToTokens;
                            let name = name_value.path.get_ident().ok_or_else(|| syn::Error::new_spanned(&name_value, "attribute name must be an identifier"))?.to_string();
                            let literal = match &name_value.value {
                                syn::Expr::Lit(syn::ExprLit { lit, .. }) => lit.to_token_stream().to_string(),
                                expression => expression.to_token_stream().to_string(),
                            }.replace('"', "");
                            match name.as_str() { $( stringify!($field) => value.$field = Some(literal), )* _ => return Err(syn::Error::new_spanned(name_value, format!("unknown attribute {name}"))) }
                        }
                        syn::Meta::Path(path) => {
                            let name = path.get_ident().ok_or_else(|| syn::Error::new_spanned(&path, "attribute name must be an identifier"))?.to_string();
                            match name.as_str() { $( stringify!($field) => value.$field = Some(String::new()), )* _ => return Err(syn::Error::new_spanned(path, format!("unknown attribute {name}"))) }
                        }
                        other => return Err(syn::Error::new_spanned(other, "unsupported attribute syntax")),
                    }
                }
                Ok(value)
            }
        }
    };
}
