use crate::{FusenAttr, fusen_crate_path};
use proc_macro::TokenStream;
use quote::quote;
use syn::{FnArg, ImplItem, ItemImpl, Meta, parse_macro_input};

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
    if contains_asset(&input.attrs)
        || input.items.iter().any(|item| match item {
            ImplItem::Fn(method) => contains_asset(&method.attrs),
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

    let self_type = &input.self_ty;
    let dispatch = input
        .items
        .iter()
        .filter_map(|item| match item {
            ImplItem::Fn(method) => Some(method),
            _ => None,
        })
        .map(|method| {
            let ident = &method.sig.ident;
            let parameters = method.sig.inputs.iter().filter_map(|input| match input {
                FnArg::Typed(input) => Some((&input.pat, &input.ty)),
                FnArg::Receiver(_) => None,
            });
            let declarations = parameters.clone().map(|(pattern, kind)| {
                quote! {
                    let #pattern: #kind = arguments
                        .next()
                        .ok_or_else(|| #runtime::error::FusenError::InvalidRequest(
                            "request argument count mismatch".into(),
                        ))?
                        .deserialize()?;
                }
            });
            let arguments = parameters.map(|(pattern, _)| pattern);
            quote! {
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
                    let result = self.#ident(#(#arguments),*).await;
                    let mut response = #runtime::protocol::fusen::response::FusenResponse::default();
                    response.protocol = context.request.protocol;
                    response.init_response(result)?;
                    context.response = Some(response);
                    return Ok(context);
                }
            }
        })
        .collect::<Vec<_>>();

    quote! {
        #[allow(non_snake_case)]
        #input

        impl #runtime::filter::FusenFilter for #self_type {
            fn call<'a>(
                &'a self,
                join_point: #runtime::filter::ProceedingJoinPoint,
            ) -> #runtime::fusen_internal_common::BoxFutureV2<
                'a,
                Result<#runtime::protocol::fusen::context::FusenContext, #runtime::error::FusenError>,
            > {
                Box::pin(async move { self.__fusen_invoke(join_point.context).await })
            }
        }

        impl #runtime::server::rpc::RpcService for #self_type {
            fn get_service_info(&self) -> #runtime::protocol::fusen::service::ServiceInfo {
                <Self as #trait_path>::__fusen_service_info()
            }
        }

        impl #self_type {
            async fn __fusen_invoke(
                &self,
                mut context: #runtime::protocol::fusen::context::FusenContext,
            ) -> Result<#runtime::protocol::fusen::context::FusenContext, #runtime::error::FusenError> {
                #(#dispatch)*
                Err(#runtime::error::FusenError::RouteNotFound(
                    context.method_info.method_name.clone(),
                ))
            }
        }
    }
    .into()
}

fn contains_asset(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| match &attr.meta {
        Meta::Path(path) => path.is_ident("asset"),
        Meta::List(list) => list.path.is_ident("asset"),
        Meta::NameValue(value) => value.path.is_ident("asset"),
    })
}
