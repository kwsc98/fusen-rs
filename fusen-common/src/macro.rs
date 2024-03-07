#[macro_export]
macro_rules! fusen_attr {
    (
    $name:ident,
    $($req:ident),*
    ) => {
        #[derive(Default)]
        struct $name {
            $(
                $req : Option<String>
            ),*
        }
        impl $name {
            fn from_attr(args: TokenStream) -> Result<$name, syn::Error> {
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated
                .parse2(args.into())
                .and_then(|args| Self::build_attr(args))
            }

            fn build_attr(args: syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>) -> Result<$name, syn::Error> {
                let mut methods_name = String::new();
                $(  
                    let mut $req  = None;
                    methods_name.push_str("`");
                    methods_name.push_str(stringify!($req));
                    methods_name.push_str("`");
                    methods_name.push_str(", ");

                )*

                for arg in args {
                    match arg {
                        syn::Meta::NameValue(namevalue) => {
                            let ident = namevalue
                                .path
                                .get_ident()
                                .ok_or_else(|| {
                                    syn::Error::new_spanned(&namevalue, "Must have specified ident")
                                })?
                                .to_string()
                                .to_lowercase();
                            let lit = match &namevalue.value {
                                syn::Expr::Lit(syn::ExprLit { lit, .. }) => lit.to_token_stream().to_string(),
                                expr => expr.to_token_stream().to_string(),
                            }
                            .replace("\"", "");
                            match ident.as_str() {
                                $(  
                                    stringify!($req) => {
                                        let _ = $req.insert(lit);
                                    }
                                )*
                                name => {
                                    let msg = format!(
                                        "Unknown attribute {} is specified; expected one of: {} ",
                                        name,&methods_name[..(methods_name.len()-2)],
                                    );
                                    return Err(syn::Error::new_spanned(namevalue, msg));
                                }
                            }
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                other,
                                "Unknown attribute inside the macro",
                            ));
                        }
                    }
                }
                Ok($name{
                    $(  
                        $req,
                    )*
                })
            }
        }
    }
}
