use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemImpl, Type};

use crate::HandlerAttr;

pub fn fusen_handler(attr: HandlerAttr, item: TokenStream) -> TokenStream {
    let org_item = parse_macro_input!(item as ItemImpl);
    let item_self = &org_item.self_ty;
    let id = match attr.id {
        Some(id) => id,
        None => {
            if let Type::Path(path) = item_self.as_ref() {
                path.path.segments[0].ident.to_string()
            } else {
                return syn::Error::new_spanned(org_item, "handler must exist impl")
                    .into_compile_error()
                    .into();
            }
        }
    };
    let item = org_item.clone();
    let trait_ident = item.trait_.unwrap().1;
    let (handler_invoker, handler_trait) = match trait_ident.segments[0].ident.to_string().as_str()
    {
        "LoadBalance" => (
            quote!(fusen_rs::handler::HandlerInvoker::LoadBalance(Box::leak(
                Box::new(self)
            )),),
            quote! {
                impl fusen_rs::handler::loadbalance::LoadBalance_ for #item_self {
                    fn select_(
                        &'static self,
                        invokers: std::sync::Arc<fusen_rs::register::ResourceInfo>,
                    ) -> fusen_rs::fusen_common::FusenFuture<Result<std::sync::Arc<fusen_rs::protocol::socket::InvokerAssets>, fusen_rs::Error>> {
                        Box::pin(async move {
                           self.select(invokers).await
                        })
                    }
                }
            },
        ),
        "Aspect" => (
            quote!(fusen_rs::handler::HandlerInvoker::Aspect(Box::leak(
                Box::new(self)
            )),),
            quote! {
                impl fusen_rs::filter::FusenFilter for #item_self {
                    fn call(
                        &'static self,
                        join_point: fusen_rs::filter::ProceedingJoinPoint,
                    ) -> fusen_rs::fusen_common::FusenFuture<Result<fusen_rs::fusen_common::FusenContext, fusen_rs::Error>> {
                        Box::pin(async move {
                            self.aroud(join_point).await
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
            .into()
        }
    };
    quote!(
        #org_item

        #handler_trait

        impl fusen_rs::handler::HandlerLoad for #item_self {
            fn load(self) -> fusen_rs::handler::Handler {
                fusen_rs::handler::Handler::new(#id.to_string(),#handler_invoker)
            }
        }
    )
    .into()
}
