use tokio::io::{AsyncReadExt, AsyncWriteExt};
use xodus::models::live::ExchangeUserTokenOutcome;
use xodus::models::secrets::Token;
use xodus::models::soap;
use xodus::models::xgameruntime::xuser::{MSATokenRequest, MSATokenResponse};
use xodus::proto::xodus::XodusMessageType;

use crate::XML_MAGIC;
use crate::simple_context::SimpleContext;

pub async fn handle(
    socket: &mut tokio::net::UnixStream,
    context: &mut SimpleContext,
) -> tokio::io::Result<()> {
    tracing::debug!("Parsing XML");
    let message_type = socket.read_u16_le().await?;
    let message_size = socket.read_u16_le().await?;
    let mut buffer = vec![0; message_size as usize];
    tracing::debug!("Reading buffer {message_size}");
    socket.read_exact(&mut buffer).await?;
    tracing::debug!("Read buffer");
    let message_type = XodusMessageType::try_from(message_type as i32).unwrap_or_default();

    let out_buf = match parse_message(context, message_type, buffer).await {
        Ok(buf) => buf,
        Err(err) => {
            tracing::error!("Failed parsing message: {err}");
            vec![]
        }
    };

    let data = super::encode_message(XML_MAGIC, message_type as u16 + 1, out_buf);
    socket.write_all(&data).await
}

/// The service endpoint table, which says which relying party each Xbox
/// Live host wants a token for. It changes rarely, so it is fetched once.
static ENDPOINTS: tokio::sync::OnceCell<xodus::models::xbox::TitleMgtResponse> =
    tokio::sync::OnceCell::const_new();

/// Mint an `XBL3.0 x=<hash>;<token>` header for whatever service `url`
/// belongs to.
async fn xbox_live_token_for(
    context: &mut SimpleContext,
    url: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let endpoints = ENDPOINTS
        .get_or_try_init(|| xodus::api::xbox::title::get_title_management(&context.client))
        .await?;
    let endpoint = xodus::api::xbox::title::get_endpoint(url, endpoints)
        .ok_or("no Xbox Live endpoint matches that address")?;
    let relying_party = endpoint
        .relying_party
        .as_deref()
        .ok_or("that endpoint needs no token")?;

    let Token::Legacy(device) = context.tokens().get_device_sts_token()? else {
        return Err("no device token".into());
    };
    let Token::Legacy(user) = context.tokens().get_user_sts_token()? else {
        return Err("no user token".into());
    };

    let xsts = xodus::api::xbox::run(&context.client, device, user, relying_party).await;
    let expiry = xsts.not_after.timestamp();
    let payload = MSATokenResponse {
        token: xodus::api::xbox::get_xsts_auth_header(xsts),
        expiry,
        device_rps: String::new(),
        device_expiry: 0,
    };
    tracing::info!("issued an Xbox Live token for {relying_party} ({url})");
    Ok(quick_xml::se::to_string(&payload)?.into_bytes())
}

pub async fn parse_message(
    context: &mut SimpleContext,
    message_type: XodusMessageType,
    buffer: Vec<u8>,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    match message_type {
        XodusMessageType::Ping => Ok(buffer),
        XodusMessageType::MsaTokenRequest => {
            tracing::debug!("Raw buffer: {buffer:?}");
            let string_buf = std::str::from_utf8(&buffer)?;
            tracing::debug!("String buffer: {string_buf:?}");
            let req = quick_xml::de::from_str::<MSATokenRequest>(string_buf)?;

            // A title that told us where it is calling gets a token minted
            // for that service. Xbox Live issues one token per relying
            // party and refuses anything else, so this is the difference
            // between XSAPI starting and XSAPI returning 0x800701AB.
            if let Some(url) = req.url.as_deref().filter(|url| !url.is_empty()) {
                match xbox_live_token_for(context, url).await {
                    Ok(payload) => return Ok(payload),
                    Err(err) => {
                        // Fall through to the old behaviour rather than
                        // failing the call outright: some callers do get
                        // something useful from a plain MSA token.
                        tracing::warn!("no Xbox Live token for {url}: {err}");
                    }
                }
            }

            let Token::Legacy(token) = context.tokens().get_user_sts_token()? else {
                return Ok(vec![]);
            };
            let scope = if req.msa_full_trust {
                "service::user.auth.xboxlive.com::MBI_SSL"
            } else {
                "xboxlive.signin"
            };
            let device_token = context.device_token.as_ref().unwrap();
            let device_token_resp = xodus::api::live::exchange_device_token(
                &context.client,
                device_token.clone(),
                "{28C08266-F973-4AE6-FFE4-409B249F138F}".to_string(),
                "scope=service::user.auth.xboxlive.com::MBI_SSL".to_owned(),
                Some(soap::PolicyReference::token_broker()),
            )
            .await;

            let ms_device_rps_token = if let Some((Token::Compact(ms_device_token), Ok(lifetime))) =
                device_token_resp.ok().map(|t| {
                    let expiry = chrono::DateTime::parse_from_rfc3339(&t.lifetime.expires);
                    (t.into(), expiry)
                }) {
                Some((ms_device_token, lifetime.timestamp()))
            } else {
                None
            };

            let user_token = xodus::api::live::exchange_user_token(
                &context.client,
                token,
                "USERNAME".to_string(),
                device_token.clone(),
                None,
                Some("Silent".to_string()),
                req.client_id.clone(),
                &[
                    (
                        format!("scope={scope}&api-version=2.0&clientid={}", req.client_id),
                        Some(soap::PolicyReference::token_broker()),
                    ),
                    ("http://Passport.NET/tb".to_string(), None),
                ],
            )
            .await?;

            match user_token {
                ExchangeUserTokenOutcome::Issued(
                    soap::BodyContent::RequestSecurityTokenResponseCollection(mut collection),
                ) => {
                    if let Some(sts) = collection.security_tokens.pop() {
                        let address = sts.applies_to.endpoint_reference.address.clone();
                        let sts: Token = sts.into();
                        let address = if let Token::Legacy(legacy) = &sts {
                            legacy.key_name.clone().unwrap_or(address)
                        } else {
                            address
                        };
                        if let Err(err) = context.tokens().save_user_token(address, sts) {
                            tracing::warn!("Failed to persist refreshed STS token: {err}");
                        }
                    }
                    let token = collection.security_tokens.remove(0);
                    let expiry = chrono::DateTime::parse_from_rfc3339(&token.lifetime.expires)?;
                    let token: Token = token.into();
                    let Token::Compact(user_token) = token else {
                        return Ok(vec![]);
                    };
                    let payload = MSATokenResponse {
                        token: user_token,
                        expiry: expiry.timestamp(),
                        device_expiry: ms_device_rps_token.as_ref().map(|(_, r)| *r).unwrap_or(0),
                        device_rps: ms_device_rps_token
                            .map(|(t, _)| t)
                            .unwrap_or_else(String::new),
                    };
                    let payload = quick_xml::se::to_string(&payload)?;
                    Ok(payload.as_bytes().to_vec())
                }
                _ => todo!("Error handling sill sucks"),
            }
        }
        _ => Err("Unimplemented".into()),
    }
}
