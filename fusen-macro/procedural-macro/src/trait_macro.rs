use crate::{
    FusenAttr, fusen_crate_path, is_asset_attr,
    validate::{MethodResource, ParameterSource, output_type, validate_trait},
};
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemTrait, TraitItem, parse_macro_input, parse_quote};

pub fn fusen_trait(attr: FusenAttr, item: TokenStream) -> TokenStream {
    let runtime = fusen_crate_path();
    let input = parse_macro_input!(item as ItemTrait);
    let resources = match validate_trait(&input) {
        Ok(resources) => resources,
        Err(error) => return error.into_compile_error().into(),
    };
    let group = attr
        .group
        .map_or_else(|| quote!(None), |value| quote!(Some(#value)));
    let version = attr
        .version
        .map_or_else(|| quote!(None), |value| quote!(Some(#value)));
    let id = attr.id.unwrap_or_else(|| input.ident.to_string());
    let trait_ident = &input.ident;
    let vis = &input.vis;
    let client_ident = format_ident!("{}Client", trait_ident);
    let client_builder_ident = format_ident!("{}ClientBuilder", trait_ident);
    let server_ident = format_ident!("{}Server", trait_ident);
    let service_info_ident = format_ident!("__fusen_{}_service_info", trait_ident);
    let service_info = service_info_tokens(&runtime, &id, &version, &group, &resources);
    let item_trait = generated_trait(&runtime, &input, &service_info_ident);

    let client_methods = input
        .items
        .iter()
        .filter_map(|item| match item {
            TraitItem::Fn(method) => Some(method),
            _ => None,
        })
        .enumerate()
        .map(|(method_index, method)| {
            let attrs = method.attrs.iter().filter(|attr| !is_asset_attr(attr));
            let ident = &method.sig.ident;
            let parameters = method
                .sig
                .inputs
                .iter()
                .filter_map(|input| match input {
                    FnArg::Typed(parameter) => Some(parameter),
                    FnArg::Receiver(_) => None,
                })
                .collect::<Vec<_>>();
            let arguments = parameters.iter().map(|parameter| &parameter.pat);
            let argument_count = parameters.len();
            let output_type = output_type(&method.sig.output);
            quote! {
                #(#attrs)*
                pub async fn #ident(&self, #(#parameters),*) -> Result<#output_type, #runtime::error::FusenError> {
                    let mut arguments = Vec::with_capacity(#argument_count);
                    #(
                        arguments.push(
                            #runtime::__private::serde_json::to_value(&#arguments)
                                .map_err(|error| #runtime::error::FusenError::internal(
                                    "failed to serialize request argument",
                                    error,
                                ))?,
                        );
                    )*
                    let response = self.client.__invoke(
                        #runtime::__private::MethodId::__new(#method_index as u16),
                        arguments,
                    ).await?;
                    #runtime::__private::serde_json::from_value(response)
                        .map_err(|error| #runtime::error::FusenError::InvalidResponse(error.to_string()))
                }
            }
        })
        .collect::<Vec<_>>();

    quote! {
        #[doc(hidden)]
        #[allow(non_snake_case)]
        #vis fn #service_info_ident() -> &'static #runtime::__private::ServiceDescriptor {
            static SERVICE_INFO: std::sync::OnceLock<#runtime::__private::ServiceDescriptor> =
                std::sync::OnceLock::new();
            SERVICE_INFO.get_or_init(|| #service_info)
        }

        #item_trait

        #[derive(Clone)]
        #vis struct #client_ident {
            client: std::sync::Arc<#runtime::__private::ServiceClient>,
        }

        #[allow(non_snake_case)]
        impl #client_ident {
            #(#client_methods)*

            pub fn builder(runtime: &#runtime::ClientRuntime) -> #client_builder_ident {
                #client_builder_ident {
                    inner: runtime.__client_builder(#service_info_ident()),
                }
            }

            pub fn service_descriptor() -> &'static #runtime::__private::ServiceDescriptor {
                #service_info_ident()
            }
        }

        #vis struct #client_builder_ident {
            inner: #runtime::__private::ServiceClientBuilder,
        }

        impl #client_builder_ident {
            pub fn direct(mut self, endpoint: impl AsRef<str>) -> Self {
                self.inner = self.inner.direct(endpoint);
                self
            }

            pub fn discover(mut self) -> Self {
                self.inner = self.inner.discover();
                self
            }

            pub fn protocol(mut self, protocol: #runtime::contract::WireProtocol) -> Self {
                self.inner = self.inner.protocol(protocol);
                self
            }

            pub fn middleware(mut self, middleware: impl #runtime::Middleware) -> Self {
                self.inner = self.inner.middleware(middleware);
                self
            }

            pub fn router(mut self, router: impl #runtime::client::cluster::Router) -> Self {
                self.inner = self.inner.router(router);
                self
            }

            pub fn load_balancer(
                mut self,
                load_balancer: impl #runtime::client::cluster::LoadBalancer,
            ) -> Self {
                self.inner = self.inner.load_balancer(load_balancer);
                self
            }

            pub async fn connect(self) -> Result<#client_ident, #runtime::error::FusenError> {
                Ok(#client_ident {
                    client: self.inner.connect().await?,
                })
            }
        }

        #vis struct #server_ident<T> {
            inner: #runtime::__private::ServerService<T>,
        }

        impl<T> #server_ident<T> {
            pub fn new(service: T) -> Self {
                Self {
                    inner: #runtime::__private::ServerService::new(service),
                }
            }

            pub fn middleware(mut self, middleware: impl #runtime::Middleware) -> Self {
                self.inner = self.inner.middleware(middleware);
                self
            }
        }

        impl<T> #runtime::__private::IntoServerService for #server_ident<T>
        where
            T: #runtime::__private::RegisteredRpcService + 'static,
        {
            fn into_server_service(self) -> #runtime::__private::PreparedService {
                self.inner.into_server_service()
            }
        }
    }
    .into()
}

fn generated_trait(
    runtime: &proc_macro2::TokenStream,
    item: &ItemTrait,
    service_info_ident: &syn::Ident,
) -> proc_macro2::TokenStream {
    let mut generated = item.clone();
    generated.attrs.retain(|attr| !is_asset_attr(attr));
    for item in &mut generated.items {
        if let TraitItem::Fn(method) = item {
            method.attrs.retain(|attr| !is_asset_attr(attr));
            let output = output_type(&method.sig.output);
            method.sig.asyncness = None;
            method.sig.output = parse_quote!(
                -> impl std::future::Future<
                    Output = std::result::Result<#output, #runtime::error::FusenError>
                > + Send
            );
        }
    }
    let dispatch_arms = item
        .items
        .iter()
        .filter_map(|item| match item {
            TraitItem::Fn(method) => Some(method),
            _ => None,
        })
        .enumerate()
        .map(|(method_index, method)| {
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
            quote! {
                #method_index => {
                    let protocol = context.__protocol();
                    let mut arguments = context.__take_arguments()?.into_iter();
                    #(#declarations)*
                    if arguments.next().is_some() {
                        return Err(#runtime::error::FusenError::InvalidRequest(
                            "request argument count mismatch".into(),
                        ));
                    }
                    let result = Self::#ident(self #(, #arguments)*).await;
                    #runtime::__private::RpcResponse::__from_result(
                        result,
                        protocol,
                    )
                }
            }
        })
        .collect::<Vec<_>>();
    let metadata: TraitItem = parse_quote! {
        #[doc(hidden)]
        fn __fusen_service_descriptor() -> &'static #runtime::__private::ServiceDescriptor
        where
            Self: Sized,
        {
            #service_info_ident()
        }
    };
    generated.items.push(metadata);
    let dispatch: TraitItem = parse_quote! {
        #[doc(hidden)]
        fn __fusen_dispatch<'a>(
            &'a self,
            mut context: #runtime::RpcContext,
        ) -> #runtime::__private::BoxFuture<'a, #runtime::RpcResult>
        where
            Self: Sized + Sync,
        {
            Box::pin(async move {
                match context.method_id().get() as usize {
                    #(#dispatch_arms,)*
                    _ => Err(#runtime::error::FusenError::RouteNotFound(
                        context.method().to_owned(),
                    )),
                }
            })
        }
    };
    generated.items.push(dispatch);
    quote! {
        #[allow(async_fn_in_trait)]
        #[allow(non_snake_case)]
        #generated
    }
}

fn service_info_tokens(
    runtime: &proc_macro2::TokenStream,
    id: &str,
    version: &proc_macro2::TokenStream,
    group: &proc_macro2::TokenStream,
    resources: &[MethodResource],
) -> proc_macro2::TokenStream {
    let methods = resources
        .iter()
        .enumerate()
        .map(|(method_index, resource)| {
            let name = &resource.name;
            let path = &resource.path;
            let method = format_ident!("{}", resource.method);
            let parameters = resource.parameters.iter().map(|(name, source)| {
                let source = match source {
                    ParameterSource::Path => quote!(Path),
                    ParameterSource::Query => quote!(Query),
                    ParameterSource::Body => quote!(Body),
                };
                quote! {
                    #runtime::__private::ParameterDescriptor::__new(
                        #name,
                        #runtime::__private::ParameterSource::#source,
                    ).expect("macro generated a validated RPC parameter")
                }
            });
            quote! {
                #runtime::__private::MethodDescriptor::__new(
                    #runtime::__private::MethodId::__new(#method_index as u16),
                    #name,
                    #runtime::__private::http::Method::#method,
                    #path,
                    vec![#(#parameters),*],
                ).expect("macro generated a validated RPC method")
            }
        });
    quote! {{
        #runtime::__private::ServiceDescriptor::__new(
            #id,
            #version,
            #group,
            vec![#(#methods),*],
        ).expect("macro generated a validated RPC service")
    }}
}
