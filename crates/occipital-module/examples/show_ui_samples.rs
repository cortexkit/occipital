//! Regenerate the wire samples through the production executor and public-API stubs.

#![forbid(unsafe_code)]

mod support;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors/show-ui-v1/outcomes.json");
    let samples = support::samples()?;
    std::fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(&samples)?),
    )?;
    println!(
        "generated {} show-ui-v1 samples",
        samples.as_array().map_or(0, Vec::len)
    );
    Ok(())
}
