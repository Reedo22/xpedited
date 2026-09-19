use std::collections::HashSet;
use std::io::IsTerminal;

use inquire::Select;
use xodus::XBOX_LIVE_PACKAGES_PC;
use xodus::api::displaycatalog::find_products_by_id;
use xodus::models::packagespc::{PackageDetails, PackageResponse};
use xodus::models::secrets::Token;
use xodus::tokens::TokenManager;

pub async fn get_content_id(
    client: &reqwest::Client,
    product: String,
    market: Option<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut seen = HashSet::new();
    resolve_content_id(client, product, market, &mut seen).await
}

/// Products point at each other: Age of Empires II names its own edition,
/// which names Age of Empires II straight back. Without remembering where we
/// have been, following those keys never returns.
async fn resolve_content_id(
    client: &reqwest::Client,
    product: String,
    market: Option<String>,
    seen: &mut HashSet<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    // Some of these graphs are large, and a launcher that sits there
    // resolving for minutes looks exactly like one that has hung.
    const MAX_PRODUCTS: usize = 12;
    if seen.len() >= MAX_PRODUCTS {
        return Err(Box::new(std::io::Error::other(
            "gave up looking for a package after a dozen related products",
        )));
    }
    if !seen.insert(product.clone()) {
        return Err(Box::new(std::io::Error::other(
            "already followed this product",
        )));
    }
    let displaycatalog = find_products_by_id(
        client,
        product.clone(),
        market.clone().unwrap_or("neutral".to_owned()),
        vec!["en".to_string(), "neutral".to_string()],
    )
    .await?;

    let product_details = displaycatalog.product;

    let mut found_package = None;
    let mut subprods: Vec<String> = vec![];
    'o: for availability in &product_details.display_sku_availabilities {
        for package in &availability.sku.properties.packages {
            if package
                .platform_dependencies
                .iter()
                .any(|dep| dep.platform_name == "Windows.Desktop")
            {
                found_package = Some(package);
                break 'o;
            }
        }
        for availability in &availability.availabilities {
            if let Some(licensing_data) = &availability.licensing_data {
                for satisfies in &licensing_data.satisfying_entitlement_keys {
                    for entitlement_key in &satisfies.entitlement_keys {
                        let key: Vec<&str> = entitlement_key.split(":").collect();
                        if key.len() == 3 && key[0] == "big" {
                            subprods.push(key[1].to_string());
                        }
                    }
                }
            }
        }
    }
    subprods.sort();
    subprods.dedup();
    // A product lists itself among the things that satisfy it, and following
    // that would recurse until the stack ran out.
    subprods.retain(|candidate| *candidate != product);

    let Some(package) = found_package else {
        if !subprods.is_empty() {
            // Most of the catalogue reaches its package this way, and most of
            // the time nobody is watching - a launcher has no terminal to
            // prompt at. Work through the candidates instead and take the
            // first that resolves; only ask when there is somebody to ask.
            if !std::io::stdin().is_terminal() {
                for candidate in &subprods {
                    if let Ok(content_id) =
                        Box::pin(get_content_id(client, candidate.clone(), market.clone())).await
                    {
                        return Ok(content_id);
                    }
                }
                return Err(Box::new(std::io::Error::other(format!(
                    "none of the {} products that satisfy this one had a Windows.Desktop package",
                    subprods.len()
                ))));
            }

            let Ok(item) = Select::new("Select files to download", subprods)
                .with_page_size(30)
                .prompt()
            else {
                return Err(Box::new(std::io::Error::other("Selection failed")));
            };
            return Box::pin(resolve_content_id(client, item, market, seen)).await;
        }

        return Err(Box::new(std::io::Error::other(
            "Windows.Desktop package not found, if you believe this is an error, please report it",
        )));
    };

    let Some(content_id) = &package.content_id else {
        tracing::error!("ContentId not found, if you believe this is an error, please report it");
        return Err(Box::new(std::io::Error::other(
            "ContentId not found, if you believe this is an error, please report it",
        )));
    };
    Ok(content_id.to_owned())
}

pub async fn get_packages(
    client: &reqwest::Client,
    tokens: &TokenManager,
    content_id: String,
) -> Result<PackageDetails, Box<dyn std::error::Error>> {
    let dev_token = tokens.get_device_sts_token().unwrap();
    let Token::Legacy(dev_token) = dev_token else {
        return Err(Box::new(std::io::Error::other("Invalid STS token")));
    };
    let user_token = tokens.get_user_sts_token().unwrap();
    let Token::Legacy(legacy) = user_token else {
        return Err(Box::new(std::io::Error::other("Unsupported user token")));
    };

    let xsts_token =
        xodus::api::xbox::run(client, dev_token, legacy, "http://update.xboxlive.com").await;

    let response = client
        .get(format!(
            "{XBOX_LIVE_PACKAGES_PC}/GetBasePackage/{content_id}"
        ))
        .header("x-xbl-contract-version", "3")
        .header(
            "Authorization",
            xodus::api::xbox::get_xsts_auth_header(xsts_token),
        )
        .send()
        .await
        .unwrap();

    let res: PackageResponse = response.json().await.expect("Failed to get data");

    let PackageResponse::Found(package) = res else {
        return Err(Box::new(std::io::Error::other(
            "Package was not found, is it owned by the user?",
        )));
    };
    Ok(package)
}
