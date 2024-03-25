use fusen_common::fusen_attr;
use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use std::collections::HashMap;
use syn::{
    parse::Parser, parse_macro_input, token::Struct, Attribute, Data, DeriveInput, Meta, Type,
};

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
pub fn resource(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_derive(UrlConfig)]
pub fn url_config(org: TokenStream) -> TokenStream {
    let org_item = parse_macro_input!(org as DeriveInput);
    eprintln!("{:#?}", org_item);
    let id = &org_item.ident;
    let fields = if let Data::Struct(data) = org_item.data {
        let mut vec = vec![];
        for field in data.fields {
            if let Type::Path(type_) = &field.ty {
                vec.push((field.ident.to_token_stream(), type_.to_token_stream()));
            } else {
                return syn::Error::new_spanned(
                    &field.ty,
                    format!("{} must be field", &field.ty.to_token_stream().to_string()),
                )
                .into_compile_error()
                .into();
            }
        }
    } else {
        return syn::Error::new_spanned(org_item, "UrlConfig must label to struct")
            .into_compile_error()
            .into();
    };


    let token = quote! {
        impl UrlConfig for id {
            pub fn form(url : &str) -> Result<Self,fusen_common::Error> {
                let info : Vec<&str> = url.split("://").collect();
                if info[0] != "urlConfig" {
                   return Err(format!("err url config {}",url).into());
                }
                let info : Vec<&str> = url.split("?").collect();
                if info[0] != stringify!(id) {
                    return Err(format!("err url config {}",url).into());
                }
                let info : Vec<&str> = url.split("&").collect();
                let mut map  = HashMap::new();
                for item in info {
                    let item : Vec<&str> = url.split("=").collect();
                    map.insert(item[0],item[1]);
                }
                
                Self {
                    #(
                        #fields.0 : match map.get("ds") {
                                           None => {
                                              if <#fields.1 as std::any::TypeId>::of::<Result<(), ()>>() == std::any::TypeId::of::<#fields.1>() {
                                                   None
                                              }else{
                                                   return Err(format!("err url config not find {}",#(fields.0)).into());
                                              }
                                           }
                                           Some(data) => {
                                               #(fields.1)::from(data)
                                           }
                                       }
                    ),*
                }
            }

            pub fn to_url(&self) -> &str {

            }
        }
    };
    token.into()
}

trait UrlConfig {
    fn form(url: &str) -> Result<Self, fusen_common::Error>;

    fn to_url(&self) -> &str;
}

fn get_resource_by_attrs(attrs: &Vec<Attribute>) -> Result<ResourceAttr, syn::Error> {
    for attr in attrs {
        if let Meta::List(list) = &attr.meta {
            if let Some(segment) = list.path.segments.first() {
                if &segment.ident.to_string() == &"resource" {
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
    version
}
