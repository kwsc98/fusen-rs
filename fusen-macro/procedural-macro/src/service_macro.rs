use crate::{FusenAttr, fusen_crate_path, is_asset_attr, validate::validate_signature};
use proc_macro::TokenStream;
use quote::quote;
use syn::{ImplItem, ItemImpl, parse_macro_input};

pub fn fusen_service(attr: FusenAttr, item: TokenStream) -> TokenStream {
    let runtime = fusen_crate_path();
    let input = parse_macro_input!(item as ItemImpl);
    if attr.id.is_some() || attr.version.is_some() || attr.group.is_some() {
        return syn::Error::new_spanned(
            &input,
            "service id, version, group, method, and path metadata belong on the fusen_trait",
        )
        .into_compile_error()
        .into();
    }
    let Some((trait_path, _)) = &input.trait_ else {
        return syn::Error::new_spanned(&input, "fusen_service requires a trait implementation")
            .into_compile_error()
            .into();
    };
    if input.attrs.iter().any(is_asset_attr)
        || input.items.iter().any(|item| match item {
            ImplItem::Fn(method) => method.attrs.iter().any(is_asset_attr),
            _ => false,
        })
    {
        return syn::Error::new_spanned(
            &input,
            "asset metadata must be declared once on the fusen_trait, not its implementation",
        )
        .into_compile_error()
        .into();
    }
    for method in input.items.iter().filter_map(|item| match item {
        ImplItem::Fn(method) => Some(method),
        _ => None,
    }) {
        if let Err(error) = validate_signature(&method.sig) {
            return error.into_compile_error().into();
        }
    }

    let self_type = &input.self_ty;
    let (impl_generics, _, where_clause) = input.generics.split_for_impl();

    quote! {
        #[allow(non_snake_case)]
        #input

        impl #impl_generics #runtime::__private::RpcService for #self_type #where_clause {
            fn call<'a>(
                &'a self,
                context: #runtime::RpcContext,
            ) -> #runtime::__private::BoxFuture<'a, #runtime::RpcResult> {
                <Self as #trait_path>::__fusen_dispatch(self, context)
            }
        }

        impl #impl_generics #runtime::__private::RpcServiceInfo for #self_type #where_clause {
            fn service_descriptor(&self) -> &'static #runtime::__private::ServiceDescriptor {
                <Self as #trait_path>::__fusen_service_descriptor()
            }
        }

        impl #impl_generics #runtime::__private::IntoServerService for #self_type #where_clause {
            fn into_server_service(self) -> #runtime::__private::PreparedService
            where
                Self: 'static,
            {
                #runtime::__private::ServerService::new(self).into_server_service()
            }
        }
    }
    .into()
}
