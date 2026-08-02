use crate::{args::ServiceArgs, runtime_path, validate};
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{Ident, ItemTrait, Lifetime, ReturnType, TraitItem, parse_macro_input, parse_quote};

struct GeneratedBindings {
    inner: Ident,
    descriptor: Ident,
    handler_type: Ident,
    handler: Ident,
    invocation: Ident,
    method_id: Ident,
    response: Ident,
    arguments: Ident,
    selector: Ident,
    methods: Ident,
    error: Ident,
    runtime: Ident,
    interceptor: Ident,
    dispatch_lifetime: Lifetime,
}

impl GeneratedBindings {
    fn new() -> Self {
        Self {
            inner: private_ident("__fusen_inner"),
            descriptor: private_ident("__FUSEN_DESCRIPTOR"),
            handler_type: private_ident("__FusenHandler"),
            handler: private_ident("__fusen_handler"),
            invocation: private_ident("__fusen_invocation"),
            method_id: private_ident("__fusen_method_id"),
            response: private_ident("__fusen_response"),
            arguments: private_ident("__fusen_arguments"),
            selector: private_ident("__fusen_selector"),
            methods: private_ident("__fusen_methods"),
            error: private_ident("__fusen_error"),
            runtime: private_ident("__fusen_runtime"),
            interceptor: private_ident("__fusen_interceptor"),
            dispatch_lifetime: Lifetime::new("'__fusen_dispatch", Span::mixed_site()),
        }
    }
}

fn private_ident(name: &str) -> Ident {
    Ident::new(name, Span::mixed_site())
}

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
    let bindings = GeneratedBindings::new();
    let generated_trait = generated_trait(&item);
    let descriptor = descriptor(&interface, &abi, &bindings);
    let client_methods = client_methods(&item, &interface, &abi, &bindings);
    let dispatch_arms = dispatch_arms(&interface, trait_ident, &bindings);
    let GeneratedBindings {
        inner,
        descriptor: descriptor_binding,
        handler_type,
        handler,
        invocation,
        method_id,
        runtime: runtime_binding,
        interceptor,
        dispatch_lifetime,
        ..
    } = &bindings;

    Ok(quote! {
        #generated_trait

        #[doc = concat!("Generated client for [`", stringify!(#trait_ident), "`].")]
        #[derive(Clone)]
        #visibility struct #client_ident {
            #inner: #abi::ServiceClient,
        }

        impl #client_ident {
            fn __descriptor() -> ::core::result::Result<
                &'static #abi::ServiceDescriptor,
                ::std::string::String,
            > {
                static #descriptor_binding: ::std::sync::OnceLock<
                    ::core::result::Result<#abi::ServiceDescriptor, ::std::string::String>
                > = ::std::sync::OnceLock::new();
                #descriptor_binding
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
            pub fn builder(#runtime_binding: &#abi::ClientRuntime) -> #abi::ClientBuilder<Self> {
                #abi::ClientBuilder::new(
                    #runtime_binding,
                    Self::__descriptor,
                    |#inner| Self { #inner },
                )
            }
        }

        impl #trait_ident for #client_ident {
            #(#client_methods)*
        }

        #[doc = concat!("Server adapter for implementations of [`", stringify!(#trait_ident), "`].")]
        #visibility struct #server_ident<#handler_type> {
            #inner: #abi::ServerService<#handler_type>,
        }

        impl<#handler_type> #server_ident<#handler_type>
        where
            #handler_type: #trait_ident + ::core::marker::Send + ::core::marker::Sync + 'static,
        {
            /// Wraps a direct implementation of the interface trait.
            pub fn new(#handler: #handler_type) -> Self {
                Self {
                    #inner: #abi::ServerService::new(
                        #handler,
                        #client_ident::__descriptor,
                        Self::__dispatch,
                    ),
                }
            }

            /// Adds an interface-local head interceptor.
            pub fn head_interceptor(mut self, #interceptor: impl #abi::Interceptor) -> Self {
                self.#inner = self.#inner.head_interceptor(#interceptor);
                self
            }

            /// Adds an interface-local decoded-call interceptor.
            pub fn interceptor(mut self, #interceptor: impl #abi::Interceptor) -> Self {
                self.#inner = self.#inner.interceptor(#interceptor);
                self
            }

            fn __dispatch<#dispatch_lifetime>(
                #handler: &#dispatch_lifetime #handler_type,
                mut #invocation: #abi::ServerInvocation,
            ) -> #abi::InterceptorFuture<#dispatch_lifetime> {
                ::std::boxed::Box::pin(async move {
                    let #method_id = #invocation.method_id();
                    match #method_id.get() {
                        #(#dispatch_arms,)*
                        _ => Err(#abi::method_not_found(#method_id)),
                    }
                })
            }
        }

        impl<#handler_type> #abi::IntoServerService for #server_ident<#handler_type>
        where
            #handler_type: #trait_ident + ::core::marker::Send + ::core::marker::Sync + 'static,
        {
            fn into_server_service(self) -> #abi::PreparedService {
                self.#inner.into_prepared()
            }
        }
    })
}

fn generated_trait(item: &ItemTrait) -> proc_macro2::TokenStream {
    let mut generated = item.clone();
    for trait_item in &mut generated.items {
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
            input.attrs.retain(|attribute| {
                !validate::is_param_attr(attribute)
                    && !crate::sensitive::is_sensitive_attr(attribute)
            });
        }
        let ReturnType::Type(_, output) = &method.sig.output else {
            unreachable!("the interface validator required a service invocation result")
        };
        let output = output.clone();
        method.sig.asyncness = None;
        method.sig.output = parse_quote!(
            -> impl ::core::future::Future<
                Output = #output
            > + ::core::marker::Send
        );
    }
    quote!(#generated)
}

fn descriptor(
    interface: &validate::Service,
    abi: &proc_macro2::TokenStream,
    bindings: &GeneratedBindings,
) -> proc_macro2::TokenStream {
    let selector_binding = &bindings.selector;
    let methods_binding = &bindings.methods;
    let error = &bindings.error;
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
        let mapping = &method.http;
        let http_method = format_ident!("{}", mapping.method);
        let path = &mapping.path;
        let fields = method.parameters.iter().filter_map(|parameter| {
            if parameter.source == validate::ParameterSource::Context {
                return None;
            }
            let name = &parameter.wire_name;
            let source = match parameter.source {
                validate::ParameterSource::Path => quote!(Path),
                validate::ParameterSource::Query => quote!(Query),
                validate::ParameterSource::Header => quote!(Header),
                validate::ParameterSource::Cookie => quote!(Cookie),
                validate::ParameterSource::BodyField => quote!(BodyField),
                validate::ParameterSource::Body => quote!(Body),
                validate::ParameterSource::QueryMap => quote!(QueryMap),
                validate::ParameterSource::HeaderMap => quote!(HeaderMap),
                validate::ParameterSource::Context => unreachable!(),
            };
            let repeated = parameter.repeated;
            Some(quote! {
                #abi::ArgumentField::new(
                    #name,
                    #abi::ArgumentSource::#source,
                    #repeated,
                )
            })
        });
        let sensitive_arguments = method.parameters.iter().filter_map(|parameter| {
            if parameter.source == validate::ParameterSource::Context {
                return None;
            }
            let name = &parameter.wire_name;
            let kind = &parameter.kind;
            let resolver = match &parameter.sensitivity {
                Some(crate::sensitive::SensitiveOverride::Kind(sensitivity)) => quote! {
                    || #abi::SensitiveShape::Kind(
                        #abi::SensitivityKind::new(#sensitivity)
                            .expect("the interface macro validated the sensitivity kind")
                    )
                },
                Some(crate::sensitive::SensitiveOverride::Opaque) => {
                    quote!(|| #abi::SensitiveShape::Opaque)
                }
                None => quote!(<#kind as #abi::SensitiveFields>::sensitive_shape),
            };
            Some(quote! {
                #abi::SensitiveArgument::new(#name, #resolver)
            })
        });
        let response = &method.response;
        let consumes = &mapping.consumes;
        let produces = &mapping.produces;
        let http = quote! {
            #abi::http_method(
                #abi::http::Method::#http_method,
                #path,
                #consumes,
                #produces,
                &[#(#fields),*],
            )?
        };
        quote! {
            #abi::MethodDescriptor::new(
                #abi::MethodId::new(#index as u16),
                #identity,
                #http,
            )
            .map_err(|#error| #error.to_string())?
            .with_sensitivity(#abi::MethodSensitivity::new(
                ::std::vec![#(#sensitive_arguments),*],
                Some(<#response as #abi::SensitiveFields>::sensitive_shape),
            ))
        }
    });
    quote! {{
        let #selector_binding = #abi::ServiceSelector::new(#name, #group, #version)
            .map_err(|#error| #error.to_string())?;
        let #methods_binding = ::std::vec![#(#methods),*];
        #abi::ServiceDescriptor::new(#selector_binding, #methods_binding)
            .map_err(|#error| #error.to_string())
    }}
}

fn client_methods(
    item: &ItemTrait,
    interface: &validate::Service,
    abi: &proc_macro2::TokenStream,
    bindings: &GeneratedBindings,
) -> Vec<proc_macro2::TokenStream> {
    let inner = &bindings.inner;
    let arguments_binding = &bindings.arguments;
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
                .find(|parameter| parameter.source == validate::ParameterSource::Context)
                .map_or_else(
                    || quote!(#abi::Call::new()),
                    |parameter| {
                        let ident = &parameter.ident;
                        quote!(#ident)
                    },
                );
            let arguments = method.parameters.iter().filter_map(|parameter| {
                if parameter.source == validate::ParameterSource::Context {
                    return None;
                }
                let ident = &parameter.ident;
                let name = &parameter.wire_name;
                Some(quote! {
                    #arguments_binding.insert(
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
                ) -> ::core::result::Result<#abi::Response<#response>, #abi::Error> {
                    self.#inner
                        .invoke::<#response, _>(
                            #abi::MethodId::new(#index as u16),
                            #call,
                            move || {
                                let mut #arguments_binding = #abi::Arguments::new();
                                #(#arguments)*
                                Ok(#arguments_binding)
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
    bindings: &GeneratedBindings,
) -> Vec<proc_macro2::TokenStream> {
    let handler_type = &bindings.handler_type;
    let handler = &bindings.handler;
    let invocation = &bindings.invocation;
    let response = &bindings.response;
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
                if parameter.source == validate::ParameterSource::Context {
                    quote! {
                        let #ident: #kind = #invocation.call();
                    }
                } else {
                    let name = &parameter.wire_name;
                    let text_encoded = parameter.text_encoded;
                    quote! {
                        let #ident: #kind = #invocation.decode_argument(
                            #name,
                            #text_encoded,
                        )?;
                    }
                }
            });
            let arguments = method.parameters.iter().map(|parameter| &parameter.ident);
            quote! {
                #method_id => {
                    #(#declarations)*
                    #invocation.finish_arguments()?;
                    let #response = <#handler_type as #trait_ident>::#ident(
                        #handler,
                        #(#arguments),*
                    ).await?;
                    #invocation.encode_response(#response)
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
                    method = "GET",
                    path = "/users/{id}"
                )]
                async fn get(
                    &self,
                    id: String,
                    expand: Option<bool>,
                ) -> Result<Response<User>, Error>;
            }
        })
        .unwrap();
        let expansion = expand_tokens(args, item).unwrap().to_string();
        assert!(expansion.contains("__macro :: v1"));
        assert!(expansion.contains("ClientBuilder < Self >"));
        assert!(expansion.contains("impl UserApi for UserApiClient"));
        assert!(!expansion.contains("UserApiClientBuilder"));
        assert!(expansion.contains("UserApiServer"));
        assert!(expansion.contains("__macro :: v1 :: Interceptor"));
        assert!(expansion.contains("head_interceptor"));
        assert!(expansion.contains("__macro :: v1 :: Response < User >"));
        assert!(expansion.contains("__macro :: v1 :: Error"));
    }
}
