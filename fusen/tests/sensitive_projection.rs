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
    ) -> Result<fusen_rs::RpcResponse<LoginResponse>, fusen_rs::RpcError>;
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

    let SensitiveShape::Fields(request) = sensitivity.arguments()[0].shape() else {
        panic!("request DTO should expose named fields")
    };
    assert_eq!(
        request.iter().map(|field| field.name()).collect::<Vec<_>>(),
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
    let SensitiveShape::Fields(profile) = profile() else {
        panic!("sequence elements should inherit the profile shape")
    };
    assert_eq!(
        profile.iter().map(|field| field.name()).collect::<Vec<_>>(),
        ["displayName", "phoneNumber"]
    );
    assert!(matches!(
        sensitivity.arguments()[1].shape(),
        SensitiveShape::Kind(SensitivityKind::IDENTIFIER)
    ));

    let SensitiveShape::Fields(response) = sensitivity.response_shape().unwrap() else {
        panic!("response DTO should expose named fields")
    };
    assert_eq!(
        response
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
