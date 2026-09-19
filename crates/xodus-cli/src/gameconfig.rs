//! Reading the handful of facts a package states about itself in
//! `MicrosoftGame.config`.

use std::path::Path;

/// Packages are inconsistent about the case of this file - Wolfenstein 3D
/// ships `MicrosoftGame.config` and Minecraft `MicrosoftGame.Config` - so try
/// what is actually on disk rather than a fixed name.
pub fn read(source: &Path) -> Option<String> {
    for entry in std::fs::read_dir(source).ok()?.flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case("MicrosoftGame.config")
        {
            return std::fs::read_to_string(entry.path())
                .ok()
                .map(|xml| strip_comments(&xml));
        }
    }
    None
}

/// The config carries commented out examples of the very fields we are
/// looking for - LIMBO ships a `**REPLACE WITH STOREID**` placeholder - so
/// the comments have to go before anything is read out of it.
pub fn strip_comments(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Every value of a repeated tag, in the order they appear.
pub fn tag_values(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = vec![];
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        rest = &rest[start + open.len()..];
        let Some(end) = rest.find(&close) else { break };
        out.push(rest[..end].trim().to_string());
        rest = &rest[end + close.len()..];
    }
    out
}

pub fn attr_value(xml: &str, element: &str, attribute: &str) -> Option<String> {
    let open = format!("<{element}");
    let key = format!("{attribute}=\"");
    let mut offset = 0;

    while let Some(found) = xml[offset..].find(&open) {
        let start = offset + found;
        offset = start + open.len();

        // `<Executable` is also the start of `<ExecutableList`, so the name
        // has to end where the element name ends.
        let tail = &xml[offset..];
        if tail.chars().next().is_some_and(|c| c.is_alphanumeric()) {
            continue;
        }

        let Some(close) = tail.find('>') else { break };
        let attributes = &tail[..close];
        if let Some(value) = attributes.find(&key) {
            let value = value + key.len();
            if let Some(end) = attributes[value..].find('"') {
                return Some(attributes[value..value + end].to_string());
            }
        }
    }
    None
}

/// The executable the package says is the game. Several packages ship more
/// than one - SUPERHOT has its Unity crash handler beside it - so guessing
/// picks the wrong one.
pub fn executable(source: &Path) -> Option<String> {
    attr_value(&read(source)?, "Executable", "Name")
}

/// The store id, ignoring placeholders. SUPERHOT states `000000000000`
/// before it states the real one.
pub fn store_id(xml: &str) -> Option<String> {
    tag_values(xml, "StoreId").into_iter().find(|id| {
        id.len() == 12
            && id.chars().all(|c| c.is_ascii_alphanumeric())
            && id.chars().any(|c| c != '0')
    })
}
