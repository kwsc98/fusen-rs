use proc_macro::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{parse_macro_input, Data, DeriveInput};

pub fn data(item: TokenStream) -> TokenStream {
    let org_item = parse_macro_input!(item as DeriveInput);
    let ident = &org_item.ident;
    let (generics_0, generics_1, generics_2) = org_item.generics.split_for_impl();
    let Data::Struct(data_struct) = &org_item.data else {
        return syn::Error::new_spanned(org_item.to_token_stream(), "builder must label to struct")
            .into_compile_error()
            .into();
    };
    let fields_builder = data_struct.fields.iter().fold(vec![], |mut vec, e| {
        let ident = e.ident.as_ref().unwrap();
        let _type = e.ty.to_token_stream();
        let mut ident_name = ident.to_string();
        if ident_name.starts_with("r#") {
            ident_name = ident_name[2..ident_name.len()].to_string();
        }
        let get_name = format_ident!("get_{}", ident_name);
        let get_mut_name = format_ident!("get_mut_{}", ident_name);
        let set_name = format_ident!("set_{}", ident_name);

        vec.push(quote!(
            pub fn #ident(mut self,#ident : #_type) -> Self {
                self.#ident = #ident;
                self
            }
            pub fn #get_name(&self) -> &#_type {
                &self.#ident
            }
            pub fn #get_mut_name(&mut self) -> &mut #_type {
                &mut self.#ident
            }
            pub fn #set_name(&mut self,#ident : #_type) -> &mut #_type {
                self.#ident = #ident;
                &mut self.#ident
            }
        ));
        vec
    });
    let token = quote! {
        impl #generics_0 #ident #generics_1
        #generics_2
        {

            #(#fields_builder)*

        }
    };
    token.into()
}
