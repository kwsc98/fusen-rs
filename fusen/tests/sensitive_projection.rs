//! Public contract coverage for derived request and response sensitivity metadata.

use fusen_rs::{SensitiveFields, SensitiveShape, SensitivityKind, interface};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, SensitiveFields)]
#[serde(rename_all = "camelCase")]
struct Profile {
    #[sensitive(kind = "public")]
    display_name: String,
    #[sensitive(kind = "phone")]
    phone_number: String,
}

#[derive(Serialize, Deserialize, SensitiveFields)]
struct LoginRequest {
    #[sensitive(kind = "credential")]
    password: String,
    profiles: Vec<Profile>,
    remember_me: bool,
}

#[derive(Serialize, Deserialize, SensitiveFields)]
struct LoginResponse {
    #[serde(rename = "userId")]
    #[sensitive(kind = "identifier")]
    user_id: String,
    #[sensitive(kind = "token")]
    access_token: String,
}

#[derive(Serialize, Deserialize, SensitiveFields)]
#[allow(dead_code)]
struct DirectionalNames {
    #[serde(
        rename(serialize = "outbound", deserialize = "inbound"),
        alias = "legacy",
        alias = "legacy_v2"
    )]
    #[sensitive(kind = "public")]
    value: String,
    #[serde(skip_serializing)]
    #[sensitive(kind = "secret")]
    inbound_only: String,
    #[serde(skip_deserializing)]
    #[sensitive(kind = "secret")]
    outbound_only: String,
}

#[interface(name = "sensitive-metadata-contract")]
#[allow(dead_code)]
trait SensitiveMetadataContract {
    #[fusen_rs::method(method = "POST", path = "/tenants/{tenant_id}/login")]
    async fn login(
        &self,
        #[param(body)] request: LoginRequest,
        #[param(path)]
        #[sensitive(kind = "identifier")]
        tenant_id: String,
    ) -> Result<fusen_rs::Response<LoginResponse>, fusen_rs::Error>;
}

#[test]
fn interface_discovers_derived_request_and_response_shapes() {
    fn assert_contract<T: SensitiveMetadataContract>() {}
    assert_contract::<SensitiveMetadataContractClient>();

    let descriptor = SensitiveMetadataContractClient::descriptor().unwrap();
    let sensitivity = descriptor.methods()[0].sensitivity().unwrap();
    assert_eq!(
        sensitivity
            .arguments()
            .iter()
            .map(|argument| argument.name())
            .collect::<Vec<_>>(),
        ["request", "tenant_id"]
    );

    let SensitiveShape::Fields {
        serialize: request,
        deserialize: request_input,
    } = sensitivity.arguments()[0].shape()
    else {
        panic!("request DTO should expose named fields")
    };
    assert_eq!(
        request.iter().map(|field| field.name()).collect::<Vec<_>>(),
        ["password", "profiles", "remember_me"]
    );
    assert_eq!(
        request_input
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>(),
        ["password", "profiles", "remember_me"]
    );
    assert!(matches!(
        request[0].shape(),
        SensitiveShape::Kind(SensitivityKind::CREDENTIAL)
    ));
    assert!(matches!(request[2].shape(), SensitiveShape::Opaque));

    let SensitiveShape::Sequence(profile) = request[1].shape() else {
        panic!("nested container should preserve its sequence shape")
    };
    let SensitiveShape::Fields {
        serialize: profile,
        deserialize: profile_input,
    } = profile()
    else {
        panic!("sequence elements should inherit the profile shape")
    };
    assert_eq!(
        profile.iter().map(|field| field.name()).collect::<Vec<_>>(),
        ["displayName", "phoneNumber"]
    );
    assert_eq!(
        profile_input
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>(),
        ["displayName", "phoneNumber"]
    );
    assert!(matches!(
        sensitivity.arguments()[1].shape(),
        SensitiveShape::Kind(SensitivityKind::IDENTIFIER)
    ));

    let SensitiveShape::Fields {
        serialize: response,
        deserialize: response_input,
    } = sensitivity.response_shape().unwrap()
    else {
        panic!("response DTO should expose named fields")
    };
    assert_eq!(
        response
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>(),
        ["userId", "access_token"]
    );
    assert_eq!(
        response_input
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>(),
        ["userId", "access_token"]
    );
    assert!(matches!(
        response[1].shape(),
        SensitiveShape::Kind(SensitivityKind::TOKEN)
    ));
}

#[test]
fn derive_tracks_directional_serde_names_aliases_and_skips() {
    let SensitiveShape::Fields {
        serialize,
        deserialize,
    } = DirectionalNames::sensitive_shape()
    else {
        panic!("directional DTO should expose named fields")
    };

    assert_eq!(
        serialize
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>(),
        ["outbound", "outbound_only"]
    );
    assert_eq!(
        deserialize
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>(),
        ["inbound", "legacy", "legacy_v2", "inbound_only"]
    );
}
