use serde::Deserialize;

/// The PC Game Pass catalog. This is the same list the Xbox app shows, and it
/// is public - no sign in, and no entitlement of any kind is needed to read
/// it. What a given account may actually download is a separate question.
const PC_GAME_PASS_SIGL: &str = "fdd9e2a7-0fee-49f6-ad69-4354098401ff";

#[derive(Debug, Deserialize)]
struct SiglEntry {
    /// The first element of the response describes the list itself and
    /// carries no product id.
    id: Option<String>,
}

pub async fn pc_catalog(client: &reqwest::Client, market: &str) -> reqwest::Result<Vec<String>> {
    let response = client
        .get(format!(
            "https://catalog.gamepass.com/sigls/v2?id={PC_GAME_PASS_SIGL}&language=en-us&market={market}"
        ))
        .send()
        .await?;
    let entries: Vec<SiglEntry> = response.error_for_status()?.json().await?;
    Ok(entries.into_iter().filter_map(|entry| entry.id).collect())
}
