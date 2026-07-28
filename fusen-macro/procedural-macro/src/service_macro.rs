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
    let service = validate::validate(args, &item)?;
    let runtime = runtime_path();
    let abi = quote!(#runtime::__macro);
    let trait_ident = &item.ident;
    let visibility = &item.vis;
    let client_ident = format_ident!("{}Client", trait_ident);
    let client_builder_ident = format_ident!("{}ClientBuilder", trait_ident);
    let server_ident = format_ident!("{}Server", trait_ident);
    let result_contracts = result_contracts(&service, &abi);
    let generated_trait = generated_trait(&item, &service, &abi);
    let descriptor = descriptor(&service, &abi);
    let client_methods = client_methods(&item, &service, &abi);
    let dispatch_arms = dispatch_arms(&service, trait_ident, &abi);

    Ok(quote! {
        #(#result_contracts)*

        #generated_trait

        #[doc = concat!("Generated client for [`", stringify!(#trait_ident), "`].")]
        #[derive(Clone)]
        #visibility struct #client_ident {
            inner: #abi::ServiceClient,
        }

        impl #client_ident {
            fn __descriptor() -> &'static #abi::ServiceDescriptor {
                static DESCRIPTOR: ::std::sync::OnceLock<#abi::ServiceDescriptor> =
                    ::std::sync::OnceLock::new();
                DESCRIPTOR.get_or_init(|| #descriptor)
            }

            /// Returns this generated client's immutable service contract.
            pub fn descriptor() -> &'static #abi::ServiceDescriptor {
                Self::__descriptor()
            }

            /// Starts configuring a client bound to `runtime`.
            pub fn builder(runtime: &#abi::ClientRuntime) -> #client_builder_ident {
                #client_builder_ident {
                    inner: #abi::ServiceClientBuilder::new(runtime, Self::__descriptor()),
                }
            }

            #(#client_methods)*
        }

        #[doc = concat!("Connection builder for [`", stringify!(#client_ident), "`].")]
        #visibility struct #client_builder_ident {
            inner: #abi::ServiceClientBuilder,
        }

        impl #client_builder_ident {
            /// Uses one explicitly configured HTTP or HTTPS endpoint.
            pub fn direct(mut self, endpoint: impl ::core::convert::AsRef<str>) -> Self {
                self.inner = self.inner.direct(endpoint);
                self
            }

            /// Resolves providers through the runtime's registry.
            pub fn discover(mut self) -> Self {
                self.inner = self.inner.discover();
                self
            }

            /// Selects the versioned wire protocol used by this client.
            pub fn protocol(mut self, protocol: #abi::WireProtocol) -> Self {
                self.inner = self.inner.protocol(protocol);
                self
            }

            /// Adds one logical-invocation middleware.
            pub fn middleware(mut self, middleware: impl #abi::Middleware) -> Self {
                self.inner = self.inner.middleware(middleware);
                self
            }

            /// Replaces the instance router for this client.
            pub fn router(mut self, router: impl #abi::Router) -> Self {
                self.inner = self.inner.router(router);
                self
            }

            /// Replaces the load balancer for this client.
            pub fn load_balancer(mut self, load_balancer: impl #abi::LoadBalancer) -> Self {
                self.inner = self.inner.load_balancer(load_balancer);
                self
            }

            /// Activates discovery or validates the direct endpoint and returns a ready client.
            pub async fn connect(self) -> ::core::result::Result<#client_ident, #abi::ClientError> {
                Ok(#client_ident {
                    inner: self.inner.connect().await?,
                })
            }
        }

        #[doc = concat!("Server adapter for implementations of [`", stringify!(#trait_ident), "`].")]
        #visibility struct #server_ident<T> {
            inner: #abi::ServerService<T>,
        }

        impl<T> #server_ident<T>
        where
            T: #trait_ident + ::core::marker::Send + ::core::marker::Sync + 'static,
        {
            /// Wraps a direct implementation of the generated service trait.
            pub fn new(service: T) -> Self {
                Self {
                    inner: #abi::ServerService::new(
                        service,
                        #client_ident::__descriptor(),
                        Self::__dispatch,
                    ),
                }
            }

            /// Adds one server-side middleware around this service.
            pub fn middleware(mut self, middleware: impl #abi::Middleware) -> Self {
                self.inner = self.inner.middleware(middleware);
                self
            }

            fn __dispatch<'__fusen>(
                service: &'__fusen T,
                mut invocation: #abi::ServerInvocation,
            ) -> #abi::BoxFuture<'__fusen, #abi::RpcResult> {
                ::std::boxed::Box::pin(async move {
                    let method_id = invocation.method_id();
                    let mut arguments = invocation.take_arguments();
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

fn result_contracts(
    service: &validate::Service,
    abi: &proc_macro2::TokenStream,
) -> Vec<proc_macro2::TokenStream> {
    service
        .methods
        .iter()
        .map(|method| {
            let declared_result = &method.declared_result;
            let success = &method.success;
            quote! {
                const _: () = {
                    fn __fusen_assert_rpc_result(
                        value: #declared_result,
                    ) -> ::core::result::Result<#success, #abi::RpcError> {
                        value
                    }
                };
            }
        })
        .collect()
}

fn generated_trait(
    item: &ItemTrait,
    service: &validate::Service,
    abi: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let mut generated = item.clone();
    for (trait_item, method) in generated.items.iter_mut().zip(&service.methods) {
        let TraitItem::Fn(trait_method) = trait_item else {
            unreachable!("the service validator rejected associated items")
        };
        trait_method
            .attrs
            .retain(|attribute| !validate::is_method_attr(attribute));
        let success = &method.success;
        trait_method.sig.asyncness = None;
        trait_method.sig.output = parse_quote!(
            -> impl ::core::future::Future<
                Output = ::core::result::Result<#success, #abi::RpcError>
            > + ::core::marker::Send
        );
    }
    quote!(#generated)
}

fn descriptor(
    service: &validate::Service,
    abi: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let name = &service.name;
    let group = service.group.as_ref().map_or_else(
        || quote!(None),
        |group| quote!(Some(::std::string::String::from(#group))),
    );
    let version = service.version.as_ref().map_or_else(
        || quote!(None),
        |version| quote!(Some(::std::string::String::from(#version))),
    );
    let methods = service.methods.iter().enumerate().map(|(index, method)| {
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
                let parameters = method.parameters.iter().map(|parameter| {
                    let name = &parameter.wire_name;
                    let source = match parameter
                        .spring_source
                        .expect("validated Spring methods map every parameter")
                    {
                        validate::ParameterSource::Path => quote!(Path),
                        validate::ParameterSource::Query => quote!(Query),
                        validate::ParameterSource::Body => quote!(Body),
                    };
                    let cardinality = match parameter.spring_cardinality {
                        validate::ParameterCardinality::Scalar => quote!(Scalar),
                        validate::ParameterCardinality::Repeated => quote!(Repeated),
                    };
                    quote! {
                        #abi::SpringCloudParameter::new(
                            #name,
                            #abi::SpringCloudParameterSource::#source,
                            #abi::SpringCloudParameterCardinality::#cardinality,
                        ).expect("the service macro generated a validated Spring parameter")
                    }
                });
                quote! {
                    Some(
                        #abi::SpringCloudMethod::new(
                            #abi::http::Method::#http_method,
                            #path,
                            ::std::vec![#(#parameters),*],
                        ).expect("the service macro generated a validated Spring mapping")
                    )
                }
            },
        );
        quote! {
            #abi::MethodDescriptor::new(
                #abi::MethodId::new(#index as u16),
                #identity,
                #abi::Idempotency::#idempotency,
                #spring,
            ).expect("the service macro generated a validated method descriptor")
        }
    });
    quote! {
        #abi::ServiceDescriptor::new(
            #abi::ServiceSelector::new(#name, #group, #version)
                .expect("the service macro generated a validated service selector"),
            ::std::vec![#(#methods),*],
        ).expect("the service macro generated a validated service descriptor")
    }
}

fn client_methods(
    item: &ItemTrait,
    service: &validate::Service,
    abi: &proc_macro2::TokenStream,
) -> Vec<proc_macro2::TokenStream> {
    item.items
        .iter()
        .zip(&service.methods)
        .enumerate()
        .map(|(index, (trait_item, method))| {
            let TraitItem::Fn(trait_method) = trait_item else {
                unreachable!("the service validator rejected associated items")
            };
            let attributes = trait_method
                .attrs
                .iter()
                .filter(|attribute| !validate::is_method_attr(attribute));
            let ident = &method.ident;
            let success = &method.success;
            let parameters = method.parameters.iter().map(|parameter| {
                let ident = &parameter.ident;
                let kind = &parameter.kind;
                quote!(#ident: #kind)
            });
            let arguments = method.parameters.iter().map(|parameter| {
                let ident = &parameter.ident;
                let name = &parameter.wire_name;
                quote! {
                    arguments.insert(
                        ::std::string::String::from(#name),
                        #abi::encode_argument(&#ident)?,
                    );
                }
            });
            quote! {
                #(#attributes)*
                pub async fn #ident(
                    &self,
                    #(#parameters),*
                ) -> ::core::result::Result<#success, #abi::RpcError> {
                    self.inner
                        .invoke::<#success, _>(
                            #abi::MethodId::new(#index as u16),
                            move || {
                                let mut arguments = #abi::Arguments::new();
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
    service: &validate::Service,
    trait_ident: &syn::Ident,
    abi: &proc_macro2::TokenStream,
) -> Vec<proc_macro2::TokenStream> {
    service
        .methods
        .iter()
        .enumerate()
        .map(|(index, method)| {
            let method_id = index as u16;
            let ident = &method.ident;
            let declarations = method.parameters.iter().map(|parameter| {
                let ident = &parameter.ident;
                let kind = &parameter.kind;
                let name = &parameter.wire_name;
                quote! {
                    let #ident: #kind = #abi::decode_argument(&mut arguments, #name)?;
                }
            });
            let arguments = method.parameters.iter().map(|parameter| &parameter.ident);
            quote! {
                #method_id => {
                    #(#declarations)*
                    #abi::finish_arguments(&arguments)?;
                    let result = <T as #trait_ident>::#ident(service, #(#arguments),*).await?;
                    invocation.encode_result(result)
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
    fn expansion_uses_only_the_macro_abi() {
        let args = syn::parse2(quote!(name = "user", group = "prod", version = "1")).unwrap();
        let item = syn::parse2(quote! {
            pub trait UserService {
                #[method(
                    idempotency = "safe",
                    spring(
                        method = "GET",
                        path = "/users/{id}",
                        query = ["expand", "labels"]
                    )
                )]
                async fn get(
                    &self,
                    id: String,
                    expand: Option<bool>,
                    labels: Vec<String>,
                ) -> Result<User, RpcError>;
            }
        })
        .unwrap();
        let expansion = expand_tokens(args, item).unwrap().to_string();
        assert!(expansion.contains("__macro"));
        assert!(!expansion.contains("__private"));
        assert!(!expansion.contains("FusenError"));
        assert!(expansion.contains("UserServiceClient"));
        assert!(expansion.contains("UserServiceClientBuilder"));
        assert!(expansion.contains("UserServiceServer"));
        assert!(expansion.contains("SpringCloudMethod"));
        assert!(expansion.contains("SpringCloudParameterCardinality :: Repeated"));
        assert!(!expansion.contains("decode_result"));
        assert!(expansion.contains("move ||"));
    }

    #[test]
    fn expansion_adds_only_the_three_public_wrapper_types() {
        let args = syn::parse2(quote!(name = "user")).unwrap();
        let item = syn::parse2(quote! {
            pub trait UserService {
                async fn get(&self) -> Result<User, RpcError>;
            }
        })
        .unwrap();
        let expansion: syn::File = syn::parse2(expand_tokens(args, item).unwrap()).unwrap();
        let public_structs = expansion
            .items
            .iter()
            .filter_map(|item| {
                let syn::Item::Struct(item) = item else {
                    return None;
                };
                matches!(item.vis, syn::Visibility::Public(_)).then(|| item.ident.to_string())
            })
            .collect::<Vec<_>>();

        assert_eq!(
            public_structs,
            [
                "UserServiceClient",
                "UserServiceClientBuilder",
                "UserServiceServer"
            ]
        );
    }
}
