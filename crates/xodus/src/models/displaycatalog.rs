use serde::{Deserialize, Serialize};

/// `#[serde(default)]` covers a field that is absent, but the catalog also
/// sends fields that are present and explicitly null - a title with no short
/// description, for instance. Both should read as the default.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DisplayCatalogProductsResponse {
    pub product: Product,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Product {
    #[serde(default, deserialize_with = "null_as_default")]
    pub product_id: String,
    pub display_sku_availabilities: Vec<DisplaySkuAvailability>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub localized_properties: Vec<LocalizedProperties>,
}

/// The catalog answers a list of ids with a list of products, and quietly
/// leaves out any it does not recognise.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DisplayCatalogProductListResponse {
    #[serde(default, deserialize_with = "null_as_default")]
    pub products: Vec<Product>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LocalizedProperties {
    #[serde(default, deserialize_with = "null_as_default")]
    pub product_title: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub publisher_name: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub developer_name: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub short_description: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub product_description: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub images: Vec<Image>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Image {
    /// `Poster`, `BoxArt`, `Logo`, `SuperHeroArt`, `Screenshot` and so on.
    #[serde(default)]
    pub image_purpose: String,
    #[serde(default)]
    pub uri: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

impl Image {
    /// The catalog hands out protocol relative uris, which are of no use to a
    /// caller that just wants to fetch the image.
    pub fn absolute_uri(&self) -> String {
        match self.uri.strip_prefix("//") {
            Some(rest) => format!("https://{rest}"),
            None => self.uri.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DisplaySkuAvailability {
    pub sku: Sku,
    pub availabilities: Vec<Availability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Sku {
    pub properties: SkuProperties,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SkuProperties {
    /// Subscription SKUs (Game Pass and similar) carry no packages, so a
    /// missing list must not fail the whole product lookup.
    #[serde(default, deserialize_with = "null_as_default")]
    pub packages: Vec<Package>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Package {
    #[serde(default)]
    pub content_id: Option<String>,
    pub platform_dependencies: Vec<PlatformDependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlatformDependency {
    pub platform_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Availability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub licensing_data: Option<LicensingData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LicensingData {
    pub satisfying_entitlement_keys: Vec<SatisfyingEntitlementKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SatisfyingEntitlementKey {
    pub entitlement_keys: Vec<String>,
}
