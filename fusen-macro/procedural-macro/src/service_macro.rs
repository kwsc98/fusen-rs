use crate::{FusenAttr, fusen_crate_path, is_asset_attr, trait_macro::validate_signature};
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ImplItem, ItemImpl, parse_macro_input};

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
    let Some((_, trait_path, _)) = &input.trait_ else {
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

    let dispatch = match build_dispatch(&runtime, trait_path, &input) {
        Ok(dispatch) => dispatch,
        Err(error) => return error.into_compile_error().into(),
    };
    let self_type = &input.self_ty;
    let (impl_generics, _, where_clause) = input.generics.split_for_impl();

    quote! {
        #[allow(non_snake_case)]
        #input

        impl #impl_generics #runtime::filter::FusenFilter for #self_type #where_clause {
            fn call<'a>(
                &'a self,
                join_point: #runtime::filter::ProceedingJoinPoint,
            ) -> #runtime::fusen_internal_common::BoxFutureV2<
                'a,
                Result<#runtime::protocol::fusen::context::FusenContext, #runtime::error::FusenError>,
            > {
                Box::pin(async move {
                    let mut context = join_point.context;
                    #(#dispatch)*
                    Err(#runtime::error::FusenError::RouteNotFound(
                        context.method_info.method_name.clone(),
                    ))
                })
            }
        }

        impl #impl_generics #runtime::server::rpc::RpcService for #self_type #where_clause {
            fn get_service_info(&self) -> #runtime::protocol::fusen::service::ServiceInfo {
                <Self as #trait_path>::__fusen_service_info()
            }
        }
    }
    .into()
}

fn build_dispatch(
    runtime: &proc_macro2::TokenStream,
    trait_path: &syn::Path,
    input: &ItemImpl,
) -> Result<Vec<proc_macro2::TokenStream>, syn::Error> {
    input
        .items
        .iter()
        .filter_map(|item| match item {
            ImplItem::Fn(method) => Some(method),
            _ => None,
        })
        .map(|method| {
            validate_signature(&method.sig)?;
            let ident = &method.sig.ident;
            let parameters = method
                .sig
                .inputs
                .iter()
                .filter_map(|input| match input {
                    FnArg::Typed(input) => Some(&input.ty),
                    FnArg::Receiver(_) => None,
                })
                .enumerate()
                .map(|(index, kind)| (format_ident!("__fusen_argument_{index}"), kind))
                .collect::<Vec<_>>();
            let declarations = parameters.iter().map(|(argument, kind)| {
                quote! {
                    let #argument: #kind = arguments
                        .next()
                        .ok_or_else(|| #runtime::error::FusenError::InvalidRequest(
                            "request argument count mismatch".into(),
                        ))?
                        .deserialize()?;
                }
            });
            let arguments = parameters.iter().map(|(argument, _)| argument);
            Ok(quote! {
                if context.method_info.method_name == stringify!(#ident) {
                    let mut arguments = context
                        .request
                        .take_arguments(&context.method_info)?
                        .into_iter();
                    #(#declarations)*
                    if arguments.next().is_some() {
                        return Err(#runtime::error::FusenError::InvalidRequest(
                            "request argument count mismatch".into(),
                        ));
                    }
                    let result = <Self as #trait_path>::#ident(self #(, #arguments)*).await;
                    let mut response = #runtime::protocol::fusen::response::FusenResponse::default();
                    response.protocol = context.request.protocol;
                    response.init_response(result)?;
                    context.response = Some(response);
                    return Ok(context);
                }
            })
        })
        .collect()
}
