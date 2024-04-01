use fusen_common::fusen_attr;
use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse::Parser, parse_macro_input, Attribute, Data, DeriveInput, Meta};

mod server_macro;
mod trait_macro;

#[proc_macro_attribute]
pub fn fusen_trait(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = FusenAttr::from_attr(attr);
    match attr {
        Ok(attr) => trait_macro::fusen_trait(attr, item),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn fusen_server(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = FusenAttr::from_attr(attr);
    match attr {
        Ok(attr) => server_macro::fusen_server(attr, item),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn asset(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn url_config(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let org_item = parse_macro_input!(item as DeriveInput);
    let id = &org_item.ident;
    let token = quote! {

        #[fusen_macro::builder]
        #[derive(serde::Serialize, serde::Deserialize)]
        #org_item

        impl #id {
            fn from_url(url : &str) -> Result<Self,fusen_common::Error> {
                let info : Vec<&str> = url.split("://").collect();
                if info[0] != "config" {
                   return Err(format!("err url {}",url).into());
                }
                let info : Vec<&str> = info[1].split("?").collect();
                if info[0] != stringify!(#id) {
                    return Err(format!("err url {}",url).into());
                }
                fusen_common::url::from_url(info[1])
            }
        }

        impl fusen_common::url::UrlConfig for #id {
            fn to_url(&self) -> Result<String, fusen_common::Error> {
                let mut res = String::new();
                res.push_str(&("config://".to_owned() + stringify!(#id) + "?" ));
                res.push_str(&(fusen_common::url::to_url(self)?));
                Ok(res)
            }
            fn boxed(self) -> Box<dyn UrlConfig> {
                Box::new(self)
            }
        }
    };
    token.into()
}

#[proc_macro_attribute]
pub fn builder(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let org_item = parse_macro_input!(item as DeriveInput);
    let id = &org_item.ident;
    let builder = syn::Ident::new(&format!("{}Builder", id), id.span());
    let Data::Struct(data_struct) = &org_item.data else {
        return syn::Error::new_spanned(org_item.to_token_stream(), "Builder must label to Struct")
            .into_compile_error()
            .into();
    };
    let fields_builder = data_struct.fields.iter().fold(vec![], |mut vec, e| {
        let id = e.ident.as_ref().unwrap();
        let _type = e.ty.to_token_stream();
        vec.push(quote!(
            pub fn #id(mut self,value : #_type) -> Self {
                self.cache.#id = value;
                self
            }
        ));
        vec
    });
    let token = quote! {
        #[derive(Default)]
        #org_item

        impl #id {
            pub fn builder() -> #builder {
                return #builder {cache :
                   #id::default()
                };
            }
        }
        pub struct #builder {
            cache : #id
        }
        impl #builder {
            #(
               #fields_builder
            )*
            pub fn build(self) -> #id {
                self.cache
            }
        }
    };
    token.into()
}

fn get_asset_by_attrs(attrs: &Vec<Attribute>) -> Result<ResourceAttr, syn::Error> {
    for attr in attrs {
        if let Meta::List(list) = &attr.meta {
            if let Some(segment) = list.path.segments.first() {
                if &segment.ident.to_string() == &"asset" {
                    return Ok(ResourceAttr::from_attr(list.tokens.clone().into())?);
                }
            }
        }
    }
    Ok(ResourceAttr::default())
}

fusen_attr! {
    ResourceAttr,
    id,
    path,
    method
}

fusen_attr! {
    FusenAttr,
    package,
    version,
    group
}
