use std::process::ExitCode;

use xodus::models::displaycatalog::{Image, LocalizedProperties};

/// The roles a launcher actually asks for, and the catalog art that suits
/// each. The first purpose present wins, so a title missing its poster still
/// gets a portrait image rather than nothing.
pub const ROLES: &[(&str, &[&str])] = &[
    ("cover", &["Poster", "BrandedKeyArt", "BoxArt"]),
    ("square", &["BoxArt", "FeaturePromotionalSquareArt", "Logo"]),
    ("hero", &["SuperHeroArt", "TitledHeroArt", "Hero"]),
    ("logo", &["Logo"]),
];

pub fn pick<'a>(images: &'a [Image], purposes: &[&str]) -> Option<&'a Image> {
    purposes.iter().find_map(|purpose| {
        images
            .iter()
            .filter(|image| image.image_purpose.eq_ignore_ascii_case(purpose))
            // Launchers scale down more gracefully than up.
            .max_by_key(|image| image.width as u64 * image.height as u64)
    })
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "")
        .replace('\t', "\\t")
}

fn print_json(product: &str, props: &LocalizedProperties) {
    println!("{{");
    println!("  \"productId\": \"{}\",", escape(product));
    println!("  \"title\": \"{}\",", escape(&props.product_title));
    println!("  \"publisher\": \"{}\",", escape(&props.publisher_name));
    println!("  \"developer\": \"{}\",", escape(&props.developer_name));
    println!(
        "  \"description\": \"{}\",",
        escape(if props.short_description.is_empty() {
            &props.product_description
        } else {
            &props.short_description
        })
    );

    println!("  \"art\": {{");
    let art: Vec<_> = ROLES
        .iter()
        .filter_map(|(role, purposes)| pick(&props.images, purposes).map(|image| (*role, image)))
        .collect();
    for (i, (role, image)) in art.iter().enumerate() {
        let comma = if i + 1 == art.len() { "" } else { "," };
        println!(
            "    \"{role}\": {{ \"uri\": \"{}\", \"width\": {}, \"height\": {} }}{comma}",
            escape(&image.absolute_uri()),
            image.width,
            image.height
        );
    }
    println!("  }},");

    let shots: Vec<_> = props
        .images
        .iter()
        .filter(|image| image.image_purpose.eq_ignore_ascii_case("Screenshot"))
        .collect();
    println!("  \"screenshots\": [");
    for (i, image) in shots.iter().enumerate() {
        let comma = if i + 1 == shots.len() { "" } else { "," };
        println!("    \"{}\"{comma}", escape(&image.absolute_uri()));
    }
    println!("  ]");
    println!("}}");
}

fn print_human(product: &str, props: &LocalizedProperties) {
    println!("{}", props.product_title);
    println!("  product id  {product}");
    if !props.publisher_name.is_empty() {
        println!("  publisher   {}", props.publisher_name);
    }
    if !props.developer_name.is_empty() {
        println!("  developer   {}", props.developer_name);
    }
    let description = if props.short_description.is_empty() {
        &props.product_description
    } else {
        &props.short_description
    };
    if !description.is_empty() {
        let line: String = description.chars().take(150).collect();
        println!("  about       {line}");
    }

    println!("\n  art");
    for (role, purposes) in ROLES {
        match pick(&props.images, purposes) {
            Some(image) => println!(
                "    {role:<8} {:>5}x{:<5} {}",
                image.width,
                image.height,
                image.absolute_uri()
            ),
            None => println!("    {role:<8} (none)"),
        }
    }

    let shots = props
        .images
        .iter()
        .filter(|image| image.image_purpose.eq_ignore_ascii_case("Screenshot"))
        .count();
    println!("\n  {shots} screenshot(s)");
}

pub async fn run(
    client: &reqwest::Client,
    product: String,
    market: Option<String>,
    json: bool,
) -> ExitCode {
    let market = market.unwrap_or("US".to_string());

    // The catalog is public, so this works without signing in.
    let response = match xodus::api::displaycatalog::find_products_by_id(
        client,
        product.clone(),
        market,
        vec!["en-US".to_string()],
    )
    .await
    {
        Ok(response) => response,
        Err(err) => {
            eprintln!("could not look up {product}: {err}");
            return ExitCode::FAILURE;
        }
    };

    let Some(props) = response.product.localized_properties.first() else {
        eprintln!("{product} has no localized properties to report");
        return ExitCode::FAILURE;
    };

    if json {
        print_json(&product, props);
    } else {
        print_human(&product, props);
    }
    ExitCode::SUCCESS
}
