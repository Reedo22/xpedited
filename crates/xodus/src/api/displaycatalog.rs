use crate::models::displaycatalog::{
    DisplayCatalogProductListResponse, DisplayCatalogProductsResponse,
};

pub async fn find_products_by_id(
    client: &reqwest::Client,
    product: String,
    market: String,
    languages: Vec<String>,
) -> reqwest::Result<DisplayCatalogProductsResponse> {
    let langs = languages.join(",");
    let response = client.get(format!("https://displaycatalog.mp.microsoft.com/v7.0/products/{product}?market={market}&languages={langs}")).send().await?;
    let response = response.error_for_status()?;
    response.json().await
}

/// Look several products up at once. The catalog accepts a modest number of
/// ids per request, so callers are expected to chunk.
pub async fn find_products_by_ids(
    client: &reqwest::Client,
    products: &[String],
    market: &str,
    languages: &[String],
) -> reqwest::Result<DisplayCatalogProductListResponse> {
    let ids = products.join(",");
    let langs = languages.join(",");
    let response = client
        .get(format!(
            "https://displaycatalog.mp.microsoft.com/v7.0/products?bigIds={ids}&market={market}&languages={langs}"
        ))
        .send()
        .await?;
    let response = response.error_for_status()?;
    response.json().await
}
