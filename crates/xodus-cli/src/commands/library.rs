use std::process::ExitCode;

use xodus::models::live::ExchangeUserTokenOutcome;
use xodus::models::secrets::Token;
use xodus::models::soap;
use xodus::tokens::TokenManager;

const MSA_TARGET: &str = "www.microsoft.com";
const CLIENT_ID: &str = "{d6d5a677-0872-4ab0-9442-bb792fce85c5}";

/// Acquire the MSA device + user tokens used by the licensing endpoints.
pub(crate) async fn ms_tokens(
    client: &reqwest::Client,
    tokens: &TokenManager,
) -> Result<(String, String, String), String> {
    let Token::Legacy(dev_token) = tokens
        .get_device_sts_token()
        .map_err(|e| format!("no device STS token ({e:?}) - run `xodus-cli login`"))?
    else {
        return Err("unsupported device token".into());
    };
    let user = tokens
        .get_user()
        .map_err(|e| format!("no user ({e:?}) - run `xodus-cli login`"))?;
    let puid = user.puid.clone();
    let Token::Legacy(legacy) = tokens
        .get_user_sts_token()
        .map_err(|e| format!("no user STS token ({e:?}) - run `xodus-cli login`"))?
    else {
        return Err("unsupported user token".into());
    };

    let device = xodus::api::live::exchange_device_token(
        client,
        dev_token.clone(),
        CLIENT_ID.to_string(),
        MSA_TARGET.to_owned(),
        Some(soap::PolicyReference::mbi_ssl()),
    )
    .await
    .map_err(|e| format!("device token exchange failed: {e:?}"))?;

    let user_out = xodus::api::live::exchange_user_token(
        client,
        legacy,
        user.username,
        dev_token,
        None,
        Some("Silent".to_string()),
        CLIENT_ID.to_string(),
        &[(
            MSA_TARGET.to_owned(),
            Some(soap::PolicyReference::mbi_ssl()),
        )],
    )
    .await
    .map_err(|e| format!("user token exchange failed: {e:?}"))?;

    let device: Token = device.into();
    let Token::Compact(device) = device else {
        return Err("unsupported ms device token".into());
    };

    let user_token: Token = match user_out {
        ExchangeUserTokenOutcome::Fault(f) => return Err(format!("user token fault: {f:?}")),
        ExchangeUserTokenOutcome::Issued(
            soap::BodyContent::RequestSecurityTokenResponseCollection(mut c),
        ) => c.security_tokens.remove(0).into(),
        ExchangeUserTokenOutcome::Issued(soap::BodyContent::RequestSecurityTokenResponse(t)) => {
            (*t).into()
        }
        _ => return Err("unexpected user token response".into()),
    };
    let Token::Compact(user_token) = user_token else {
        return Err("unsupported ms user token".into());
    };

    Ok((device, user_token, puid))
}

/// Resolve a product id to its store title, falling back to the id itself.
async fn product_title(client: &reqwest::Client, product_id: &str, market: &str) -> String {
    match xodus::api::displaycatalog::find_products_by_id(
        client,
        product_id.to_string(),
        market.to_string(),
        vec!["en-US".to_string()],
    )
    .await
    {
        Ok(r) => r
            .product
            .localized_properties
            .first()
            .map(|p| p.product_title.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| product_id.to_string()),
        Err(_) => product_id.to_string(),
    }
}

pub async fn run(
    client: &reqwest::Client,
    tokens: &TokenManager,
    market: String,
    all: bool,
) -> ExitCode {
    let (device_token, user_token, puid) = match ms_tokens(client, tokens).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let collection = match xodus::api::collections::query_collection(
        client,
        device_token,
        user_token,
        puid,
        market.clone(),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to query collection: {e}");
            return ExitCode::FAILURE;
        }
    };

    let items: Vec<_> = collection
        .items
        .iter()
        .filter(|i| all || i.is_game())
        .collect();

    if items.is_empty() {
        println!(
            "No {} found on this account.",
            if all { "items" } else { "games" }
        );
        return ExitCode::SUCCESS;
    }

    let mut rows = Vec::with_capacity(items.len());
    for item in &items {
        let title = product_title(client, &item.product_id, &market).await;
        rows.push((
            item.product_id.clone(),
            item.product_kind.clone(),
            item.status.clone(),
            title,
        ));
    }
    rows.sort_by_key(|r| r.3.to_lowercase());

    let id_w = rows.iter().map(|r| r.0.len()).max().unwrap_or(9).max(10);
    let kind_w = rows.iter().map(|r| r.1.len()).max().unwrap_or(4).max(4);
    let st_w = rows.iter().map(|r| r.2.len()).max().unwrap_or(6).max(6);
    println!(
        "{:<id_w$}  {:<kind_w$}  {:<st_w$}  TITLE",
        "PRODUCT ID", "KIND", "STATUS"
    );
    for (id, kind, status, title) in &rows {
        println!("{id:<id_w$}  {kind:<kind_w$}  {status:<st_w$}  {title}");
    }

    let inactive = items.iter().filter(|i| !i.is_active()).count();
    if inactive > 0 {
        println!(
            "\n{inactive} of {} are not Active (revoked or expired) and cannot be downloaded.",
            items.len()
        );
    }
    println!("\nDownload one with: xodus-cli download <PRODUCT ID>");
    ExitCode::SUCCESS
}
