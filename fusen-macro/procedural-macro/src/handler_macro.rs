use crate::{HandlerAttr, fusen_crate_path};
use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemImpl, Type, parse_macro_input};

pub fn fusen_handler(attr: HandlerAttr, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemImpl);
    let runtime = fusen_crate_path();
    let self_type = &input.self_ty;
    let id = match attr.id {
        Some(id) => id,
        None => match self_type.as_ref() {
            Type::Path(path) => {
                let Some(segment) = path.path.segments.last() else {
                    return syn::Error::new_spanned(&path.path, "handler type path is empty")
                        .into_compile_error()
                        .into();
                };
                segment.ident.to_string()
            }
            _ => {
                return syn::Error::new_spanned(
                    &input.self_ty,
                    "handler implementations require a named type or an explicit id",
                )
                .into_compile_error()
                .into();
            }
        },
    };
    let Some((_, trait_path, _)) = &input.trait_ else {
        return syn::Error::new_spanned(&input, "handler requires a trait implementation")
            .into_compile_error()
            .into();
    };
    let inferred_kind = trait_path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
        .unwrap_or_default();
    let kind = match normalize_kind(attr.kind.as_deref().unwrap_or(&inferred_kind)) {
        Some(kind) => kind,
        None => {
            return syn::Error::new_spanned(
                trait_path,
                "handler kind must be `Aspect` or `LoadBalance`; specify `kind = ...` when the trait is imported with an alias",
            )
            .into_compile_error()
            .into();
        }
    };
    let (impl_generics, _, where_clause) = input.generics.split_for_impl();

    let (handler_invoker, adapter) = match kind {
        HandlerKind::LoadBalance => (
            quote!(#runtime::handler::HandlerInvoker::LoadBalance(
                std::sync::Arc::new(self)
            ),),
            quote! {
                impl #impl_generics #runtime::handler::loadbalance::LoadBalanceDyn for #self_type #where_clause {
                    fn select_dyn<'a>(
                        &'a self,
                        context: &'a #runtime::protocol::fusen::context::FusenContext,
                        invokers: std::sync::Arc<Vec<std::sync::Arc<#runtime::contract::ServiceInstance>>>,
                    ) -> #runtime::contract::BoxFuture<'a, Result<std::option::Option<std::sync::Arc<#runtime::contract::ServiceInstance>>, #runtime::error::FusenError>> {
                        Box::pin(async move {
                            <Self as #trait_path>::select(self, context, invokers).await
                        })
                    }
                }
            },
        ),
        HandlerKind::Aspect => (
            quote!(#runtime::handler::HandlerInvoker::Aspect(
                std::sync::Arc::new(self)
            ),),
            quote! {
                impl #impl_generics #runtime::filter::FusenFilter for #self_type #where_clause {
                    fn call<'a>(
                        &'a self,
                        join_point: #runtime::filter::ProceedingJoinPoint,
                    ) -> #runtime::contract::BoxFuture<'a, Result<#runtime::protocol::fusen::context::FusenContext, #runtime::error::FusenError>> {
                        Box::pin(async move {
                            <Self as #trait_path>::around(self, join_point).await
                        })
                    }
                }
            },
        ),
    };

    quote! {
        #input

        #adapter

        impl #impl_generics #runtime::handler::HandlerLoad for #self_type #where_clause {
            fn load(self) -> #runtime::handler::Handler {
                #runtime::handler::Handler {
                    id: #id.to_string(),
                    handler_invoker: #handler_invoker
                }
            }
        }
    }
    .into()
}

#[derive(Clone, Copy)]
enum HandlerKind {
    Aspect,
    LoadBalance,
}

fn normalize_kind(kind: &str) -> Option<HandlerKind> {
    match kind.to_ascii_lowercase().replace(['_', '-'], "").as_str() {
        "aspect" => Some(HandlerKind::Aspect),
        "loadbalance" => Some(HandlerKind::LoadBalance),
        _ => None,
    }
}
