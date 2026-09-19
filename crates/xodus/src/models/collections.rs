use serde::{Deserialize, Serialize};

/// Request body for the Store collections query.
///
/// The service validates this shape before authentication, so a malformed
/// request fails with a descriptive 400 rather than a 401.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionQueryRequest {
    pub beneficiaries: Vec<Beneficiary>,
    pub market: String,
    pub max_page_size: u32,
}

/// Identity the collection is queried on behalf of. Mirrors the user identity
/// used by the licensing endpoints.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Beneficiary {
    pub identity_type: String,
    pub identity_value: String,
    pub local_ticket_reference: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionQueryResponse {
    #[serde(default)]
    pub items: Vec<CollectionItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionItem {
    pub product_id: String,
    #[serde(default)]
    pub product_kind: String,
    #[serde(default)]
    pub product_family: String,
    /// `Active`, `Revoked` or `Expired`. Only `Active` items are usable.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub sku_id: String,
    #[serde(default)]
    pub ownership_type: String,
}

impl CollectionItem {
    pub fn is_active(&self) -> bool {
        self.status.eq_ignore_ascii_case("Active")
    }

    pub fn is_game(&self) -> bool {
        self.product_kind.eq_ignore_ascii_case("Game")
    }
}
