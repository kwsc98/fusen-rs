use crate::{args::ServiceArgs, runtime_path, validate};
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{ItemTrait, TraitItem, parse_macro_input, parse_quote};

pub(crate) fn expand(args: ServiceArgs, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemTrait);
    match expand_tokens(args, item) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_tokens(args: ServiceArgs, item: ItemTrait) -> syn::Result<proc_macro2::TokenStream> {
    let interface = validate::validate(args, &item)?;
    let runtime = runtime_path();
    let abi = quote!(#runtime::__macro::v1);
    let trait_ident = &item.ident;
    let visibility = &item.vis;
    let client_ident = format_ident!("{}Client", trait_ident);
    let server_ident = format_ident!("{}Server", trait_ident);
    let generated_trait = generated_trait(&item, &interface, &abi);
    let descriptor = descriptor(&interface, &abi);
    let client_methods = client_methods(&item, &interface, &abi);
    let dispatch_arms = dispatch_arms(&interface, trait_ident);

    Ok(quote! {
        #generated_trait

        #[doc = concat!("Generated client for [`", stringify!(#trait_ident), "`].")]
        #[derive(Clone)]
        #visibility struct #client_ident {
            inner: #abi::ServiceClient,
        }

        impl #client_ident {
            fn __descriptor() -> ::core::result::Result<
                &'static #abi::ServiceDescriptor,
                ::std::string::String,
            > {
                static DESCRIPTOR: ::std::sync::OnceLock<
                    ::core::result::Result<#abi::ServiceDescriptor, ::std::string::String>
                > = ::std::sync::OnceLock::new();
                DESCRIPTOR
                    .get_or_init(|| (|| #descriptor)())
                    .as_ref()
                    .map_err(::core::clone::Clone::clone)
            }

            /// Returns this generated client's validated immutable interface contract.
            pub fn descriptor() -> ::core::result::Result<
                &'static #abi::ServiceDescriptor,
                ::std::string::String,
            > {
                Self::__descriptor()
            }

            /// Starts configuring a client bound to `runtime`.
            pub fn builder(runtime: &#abi::ClientRuntime) -> #abi::ClientBuilder<Self> {
                #abi::ClientBuilder::new(
                    runtime,
                    Self::__descriptor,
                    |inner| Self { inner },
                )
            }
        }

        impl #trait_ident for #client_ident {
            #(#client_methods)*
        }

        #[doc = concat!("Server adapter for implementations of [`", stringify!(#trait_ident), "`].")]
        #visibility struct #server_ident<T> {
            inner: #abi::ServerService<T>,
        }

        impl<T> #server_ident<T>
        where
            T: #trait_ident + ::core::marker::Send + ::core::marker::Sync + 'static,
        {
            /// Wraps a direct implementation of the interface trait.
            pub fn new(handler: T) -> Self {
                Self {
                    inner: #abi::ServerService::new(
                        handler,
                        #client_ident::__descriptor,
                        Self::__dispatch,
                    ),
                }
            }

            /// Adds interface-local head middleware.
            pub fn head_middleware(mut self, middleware: impl #abi::Middleware) -> Self {
                self.inner = self.inner.head_middleware(middleware);
                self
            }

            /// Adds interface-local decoded-call middleware.
            pub fn middleware(mut self, middleware: impl #abi::Middleware) -> Self {
                self.inner = self.inner.middleware(middleware);
                self
            }

            fn __dispatch<'__fusen>(
                handler: &'__fusen T,
                mut invocation: #abi::ServerInvocation,
            ) -> #abi::MiddlewareFuture<'__fusen> {
                ::std::boxed::Box::pin(async move {
                    let method_id = invocation.method_id();
                    match method_id.get() {
                        #(#dispatch_arms,)*
                        _ => Err(#abi::method_not_found(method_id)),
                    }
                })
            }
        }

        impl<T> #abi::IntoServerService for #server_ident<T>
        where
            T: #trait_ident + ::core::marker::Send + ::core::marker::Sync + 'static,
        {
            fn into_server_service(self) -> #abi::PreparedService {
                self.inner.into_prepared()
            }
        }
    })
}

fn generated_trait(
    item: &ItemTrait,
    interface: &validate::Service,
    abi: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let mut generated = item.clone();
    for (trait_item, contract) in generated.items.iter_mut().zip(&interface.methods) {
        let TraitItem::Fn(method) = trait_item else {
            unreachable!("the interface validator rejected associated items")
        };
        method
            .attrs
            .retain(|attribute| !validate::is_method_attr(attribute));
        for input in method.sig.inputs.iter_mut().skip(1) {
            let syn::FnArg::Typed(input) = input else {
                unreachable!("the interface validator rejected extra receivers")
            };
            input
                .attrs
                .retain(|attribute| !validate::is_rpc_attr(attribute));
        }
        let response = &contract.response;
        method.sig.asyncness = None;
        method.sig.output = parse_quote!(
            -> impl ::core::future::Future<
                Output = ::core::result::Result<#abi::RpcResponse<#response>, #abi::RpcError>
            > + ::core::marker::Send
        );
    }
    quote!(#generated)
}

fn descriptor(
    interface: &validate::Service,
    abi: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let name = &interface.name;
    let group = interface.group.as_ref().map_or_else(
        || quote!(None),
        |group| quote!(Some(::std::string::String::from(#group))),
    );
    let version = interface.version.as_ref().map_or_else(
        || quote!(None),
        |version| quote!(Some(::std::string::String::from(#version))),
    );
    let methods = interface.methods.iter().enumerate().map(|(index, method)| {
        let identity = method.ident.to_string();
        let identity = identity.trim_start_matches("r#");
        let idempotency = match method.idempotency {
            validate::Idempotency::None => quote!(None),
            validate::Idempotency::Idempotent => quote!(Idempotent),
            validate::Idempotency::Safe => quote!(Safe),
        };
        let spring = method.spring.as_ref().map_or_else(
            || quote!(None),
            |spring| {
                let http_method = format_ident!("{}", spring.method);
                let path = &spring.path;
                let fields = method.parameters.iter().filter_map(|parameter| {
                    if parameter.source == validate::ParameterSource::Call {
                        return None;
                    }
                    let name = &parameter.wire_name;
                    let source = match parameter.source {
                        validate::ParameterSource::Path => quote!(Path),
                        validate::ParameterSource::Query => quote!(Query),
                        validate::ParameterSource::Body => quote!(Body),
                        validate::ParameterSource::Call => unreachable!(),
                    };
                    let repeated = parameter.repeated;
                    let parse_primitive = parameter.parse_spring_json_primitive;
                    Some(quote! {
                        #abi::RpcField::new(
                            #name,
                            #abi::RpcFieldSource::#source,
                            #repeated,
                            #parse_primitive,
                        )
                    })
                });
                quote! {
                    Some(#abi::spring_method(
                        #abi::http::Method::#http_method,
                        #path,
                        &[#(#fields),*],
                    )?)
                }
            },
        );
        quote! {
            #abi::MethodDescriptor::new(
                #abi::MethodId::new(#index as u16),
                #identity,
                #abi::Idempotency::#idempotency,
                #spring,
            ).map_err(|error| error.to_string())?
        }
    });
    quote! {{
        let selector = #abi::ServiceSelector::new(#name, #group, #version)
            .map_err(|error| error.to_string())?;
        let methods = ::std::vec![#(#methods),*];
        #abi::ServiceDescriptor::new(selector, methods).map_err(|error| error.to_string())
    }}
}

fn client_methods(
    item: &ItemTrait,
    interface: &validate::Service,
    abi: &proc_macro2::TokenStream,
) -> Vec<proc_macro2::TokenStream> {
    item.items
        .iter()
        .zip(&interface.methods)
        .enumerate()
        .map(|(index, (trait_item, method))| {
            let TraitItem::Fn(trait_method) = trait_item else {
                unreachable!("the interface validator rejected associated items")
            };
            let attributes = trait_method
                .attrs
                .iter()
                .filter(|attribute| !validate::is_method_attr(attribute));
            let ident = &method.ident;
            let response = &method.response;
            let parameters = method.parameters.iter().map(|parameter| {
                let ident = &parameter.ident;
                let kind = &parameter.kind;
                quote!(#ident: #kind)
            });
            let call = method
                .parameters
                .iter()
                .find(|parameter| parameter.source == validate::ParameterSource::Call)
                .map_or_else(
                    || quote!(#abi::RpcCall::new()),
                    |parameter| {
                        let ident = &parameter.ident;
                        quote!(#ident)
                    },
                );
            let arguments = method.parameters.iter().filter_map(|parameter| {
                if parameter.source == validate::ParameterSource::Call {
                    return None;
                }
                let ident = &parameter.ident;
                let name = &parameter.wire_name;
                Some(quote! {
                    arguments.insert(
                        ::std::string::String::from(#name),
                        #abi::encode_argument(&#ident)?,
                    );
                })
            });
            quote! {
                #(#attributes)*
                async fn #ident(
                    &self,
                    #(#parameters),*
                ) -> ::core::result::Result<#abi::RpcResponse<#response>, #abi::RpcError> {
                    self.inner
                        .invoke::<#response, _>(
                            #abi::MethodId::new(#index as u16),
                            #call,
                            move || {
                                let mut arguments = #abi::RpcArguments::new();
                                #(#arguments)*
                                Ok(arguments)
                            },
                        )
                        .await
                }
            }
        })
        .collect()
}

fn dispatch_arms(
    interface: &validate::Service,
    trait_ident: &syn::Ident,
) -> Vec<proc_macro2::TokenStream> {
    interface
        .methods
        .iter()
        .enumerate()
        .map(|(index, method)| {
            let method_id = index as u16;
            let ident = &method.ident;
            let declarations = method.parameters.iter().map(|parameter| {
                let ident = &parameter.ident;
                let kind = &parameter.kind;
                if parameter.source == validate::ParameterSource::Call {
                    quote! {
                        let #ident: #kind = invocation.rpc_call();
                    }
                } else {
                    let name = &parameter.wire_name;
                    let parse_primitive = parameter.parse_spring_json_primitive;
                    quote! {
                        let #ident: #kind = invocation.decode_argument(
                            #name,
                            #parse_primitive,
                        )?;
                    }
                }
            });
            let arguments = method.parameters.iter().map(|parameter| &parameter.ident);
            quote! {
                #method_id => {
                    #(#declarations)*
                    invocation.finish_arguments()?;
                    let response = <T as #trait_ident>::#ident(
                        handler,
                        #(#arguments),*
                    ).await?;
                    invocation.encode_response(response)
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn expansion_uses_versioned_macro_abi_and_generic_builder() {
        let args = syn::parse2(quote!(name = "user", group = "prod", version = "1")).unwrap();
        let item = syn::parse2(quote! {
            pub trait UserApi {
                #[method(
                    idempotency = "safe",
                    spring(method = "GET", path = "/users/{id}")
                )]
                async fn get(
                    &self,
                    #[rpc(path)] id: String,
                    #[rpc(query)] expand: Option<bool>,
                ) -> Result<RpcResponse<User>, RpcError>;
            }
        })
        .unwrap();
        let expansion = expand_tokens(args, item).unwrap().to_string();
        assert!(expansion.contains("__macro :: v1"));
        assert!(expansion.contains("ClientBuilder < Self >"));
        assert!(expansion.contains("impl UserApi for UserApiClient"));
        assert!(!expansion.contains("UserApiClientBuilder"));
        assert!(expansion.contains("UserApiServer"));
    }
}
