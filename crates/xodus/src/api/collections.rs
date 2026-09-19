use xal::cvlib::CorrelationVector;

use crate::models::collections::{Beneficiary, CollectionQueryRequest, CollectionQueryResponse};

pub const COLLECTIONS_QUERY_URL: &str =
    "https://collections.mp.microsoft.com/v7.0/collections/query";

/// Query the products on an account: games, apps, durables and passes.
///
/// Authentication mirrors the licensing endpoints - the MSA device token goes
/// in `Authorization`, and the MSA user token identifies the beneficiary whose
/// collection is returned.
pub async fn query_collection(
    client: &reqwest::Client,
    device_ms_token: String,
    user_ms_token: String,
    ticket_reference: String,
    market: String,
) -> reqwest::Result<CollectionQueryResponse> {
    let cv = CorrelationVector::new();
    let response = client
        .post(COLLECTIONS_QUERY_URL)
        .header("Authorization", device_ms_token)
        .header("MS-CV", cv.to_string())
        .json(&CollectionQueryRequest {
            beneficiaries: vec![Beneficiary {
                identity_type: "Msa".to_string(),
                identity_value: user_ms_token,
                local_ticket_reference: ticket_reference,
            }],
            market,
            max_page_size: 100,
        })
        .send()
        .await?;
    let response = response.error_for_status()?;
    response.json().await
}
