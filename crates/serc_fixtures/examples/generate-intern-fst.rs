//! Serialize a JSON object mapping interned strings to Python version masks.

use std::collections::BTreeMap;
use std::io;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let entries: BTreeMap<String, u64> = serde_json::from_reader(io::stdin().lock())?;
    let mut builder = fst::MapBuilder::new(io::BufWriter::new(io::stdout().lock()))?;
    for (word, versions) in entries {
        builder.insert(word, versions)?;
    }
    builder.finish()?;
    Ok(())
}
