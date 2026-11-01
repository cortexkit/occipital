//! Regenerate the portable template contract with `cargo run --locked -p
//! cortexkit-occipital-registry --example registry_vectors`. An optional directory argument
//! writes a fresh copy elsewhere for comparison. Only this example performs I/O;
//! the library remains usable by the phone without a filesystem.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;

use occipital_registry::{canon, sha256_digest, Template};

#[path = "registry_vectors/canonical.rs"]
mod canonical;
#[path = "registry_vectors/documents.rs"]
mod documents;
#[path = "registry_vectors/support.rs"]
mod support;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;
pub type Files = BTreeMap<String, Vec<u8>>;

#[derive(Clone, Copy)]
pub enum Kind {
    Json,
    Template,
}

impl Kind {
    fn canonical(self, input: &[u8]) -> Result<Vec<u8>> {
        Ok(match self {
            Self::Json => canon::canonicalize(input)?,
            Self::Template => Template::parse(input)?.to_canonical()?,
        })
    }
}

pub struct Vectors {
    pub files: Files,
}

impl Vectors {
    fn put(&mut self, name: String, bytes: Vec<u8>) -> Result<()> {
        if self.files.insert(name.clone(), bytes).is_some() {
            return Err(format!("duplicate vector file: {name}").into());
        }
        Ok(())
    }

    fn accepted(&mut self, name: &str, input: impl AsRef<[u8]>, kind: Kind) -> Result<()> {
        let input = input.as_ref();
        let canonical = kind.canonical(input)?;
        self.put(format!("{name}.input.json"), input.to_vec())?;
        self.put(
            format!("{name}.digest"),
            format!("{}\n", sha256_digest(&canonical)).into_bytes(),
        )?;
        self.put(format!("{name}.canonical.json"), canonical)
    }

    fn refused(
        &mut self,
        name: &str,
        input: impl AsRef<[u8]>,
        kind: Kind,
        expected: &str,
    ) -> Result<()> {
        let input = input.as_ref();
        let code = match kind {
            Kind::Json => canon::canonicalize(input).err().map(|e| e.code()),
            Kind::Template => Template::parse(input).err().map(|e| e.code()),
        };
        if code != Some(expected) {
            return Err(format!("{name}: expected {expected}, got {code:?}").into());
        }
        self.put(format!("{name}.input"), input.to_vec())?;
        self.put(format!("{name}.code"), format!("{expected}\n").into_bytes())
    }
}

pub fn generate() -> Result<Files> {
    let mut vectors = Vectors {
        files: Files::new(),
    };
    canonical::add(&mut vectors)?;
    documents::add(&mut vectors)?;
    let mut sums = String::new();
    for (path, bytes) in &vectors.files {
        let digest = sha256_digest(bytes);
        sums.push_str(&digest[7..]);
        sums.push_str("  ");
        sums.push_str(path);
        sums.push('\n');
    }
    vectors.put("SHA256SUMS".into(), sums.into_bytes())?;
    Ok(vectors.files)
}

pub fn write_vectors(directory: &Path) -> Result<usize> {
    let files = generate()?;
    std::fs::create_dir_all(directory)?;
    for (path, bytes) in &files {
        std::fs::write(directory.join(path), bytes)?;
    }
    Ok(files.len())
}

#[cfg(not(test))]
fn main() -> Result<()> {
    let path = std::env::args_os().nth(1).map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("test-vectors/rich-ui-registry-v1"),
        Into::into,
    );
    let count = write_vectors(&path)?;
    println!("Wrote {count} registry vector files to {}", path.display());
    Ok(())
}
