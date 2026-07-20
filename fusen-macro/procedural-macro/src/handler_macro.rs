use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemImpl, Type, parse_macro_input};

use crate::{HandlerAttr, fusen_crate_path};

pub fn fusen_handler(attr: HandlerAttr, item: TokenStream) -> TokenStream {
    let org_item = parse_macro_input!(item as ItemImpl);
    let runtime = fusen_crate_path();
    let item_self = &org_item.self_ty;
    let id = match attr.id {
        Some(id) => id,
        None => {
            if let Type::Path(path) = item_self.as_ref() {
                let Some(segment) = path.path.segments.last() else {
                    return syn::Error::new_spanned(&path.path, "handler type path is empty")
                        .into_compile_error()
                        .into();
                };
                segment.ident.to_string()
            } else {
                return syn::Error::new_spanned(org_item, "handler must exist impl")
                    .into_compile_error()
                    .into();
            }
        }
    };
    let item = org_item.clone();
    let Some(trait_ident) = item.trait_.map(|e| e.1) else {
        return syn::Error::new_spanned(org_item, "handler must exist impl")
            .into_compile_error()
            .into();
    };
    let Some(handler_trait_name) = trait_ident
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
    else {
        return syn::Error::new_spanned(trait_ident, "handler trait path is empty")
            .into_compile_error()
            .into();
    };
    let (handler_invoker, handler_trait) = match handler_trait_name.as_str() {
        "LoadBalance" => (
            quote!(#runtime::handler::HandlerInvoker::LoadBalance(
                std::sync::Arc::new(self)
            ),),
            quote! {
                impl #runtime::handler::loadbalance::LoadBalanceDyn for #item_self {
                    fn select_dyn<'a>(
                        &'a self,
                        context: &'a #runtime::protocol::fusen::context::FusenContext,
                        invokers: std::sync::Arc<Vec<std::sync::Arc<#runtime::fusen_internal_common::resource::service::ServiceResource>>>,
                    ) -> #runtime::fusen_internal_common::BoxFutureV2<'a,Result<std::option::Option<std::sync::Arc<#runtime::fusen_internal_common::resource::service::ServiceResource>>, #runtime::error::FusenError>> {
                        Box::pin(async move {
                           self.select(context,invokers).await
                        })
                    }
                }
            },
        ),
        "Aspect" => (
            quote!(#runtime::handler::HandlerInvoker::Aspect(
                std::sync::Arc::new(self)
            ),),
            quote! {
                impl #runtime::filter::FusenFilter for #item_self {
                    fn call<'a>(
                        &'a self,
                        join_point: #runtime::filter::ProceedingJoinPoint,
                    ) -> #runtime::fusen_internal_common::BoxFutureV2<'a,Result<#runtime::protocol::fusen::context::FusenContext, #runtime::error::FusenError>> {
                        Box::pin(async move {
                            self.around(join_point).await
                        })
                    }
                }
            },
        ),
        _ => {
            return syn::Error::new_spanned(
                trait_ident,
                "handler must impl 'LoadBalance', 'Aspect'",
            )
            .into_compile_error()
            .into();
        }
    };
    quote!(
        #org_item

        #handler_trait

        impl #runtime::handler::HandlerLoad for #item_self {
            fn load(self) -> #runtime::handler::Handler {
                #runtime::handler::Handler{
                    id: #id.to_string(),
                    handler_invoker: #handler_invoker
                }
            }
        }
    )
    .into()
}
