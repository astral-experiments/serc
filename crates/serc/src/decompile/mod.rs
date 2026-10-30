//! Reconstruct Python source from the instruction stream stored in a CPython cache file.
mod flow;
mod instructions;
mod source;
mod verify;

use crate::marshal::reader::read;
use crate::{CompileOptions, OptimizationLevel, PythonVersion, compile_with_options};
use std::fmt;

/// A cache file is invalid, unsupported, or could not be faithfully reconstructed.
#[derive(Debug)]
#[non_exhaustive]
pub enum DecompileError {
    /// The cache header or marshal graph could not be read.
    Read(String),
    /// The target's instruction stream could not be represented as Python source.
    Reconstruct(PythonVersion, String),
    /// Recompiling the source failed to reproduce the input code.
    Verify(PythonVersion, String),
}

impl fmt::Display for DecompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(message) => f.write_str(message),
            Self::Reconstruct(version, message) | Self::Verify(version, message) => {
                write!(f, "{version}: {message}")
            }
        }
    }
}
impl std::error::Error for DecompileError {}

/// Recover an entire module as Python source without executing the input.
/// The cache magic selects one of serc's pinned targets. An error in any
/// nested scope rejects the entire module.
pub fn decompile_pyc(bytes: &[u8]) -> Result<String, DecompileError> {
    let module = read(bytes).map_err(DecompileError::Read)?;
    let version = module.version;
    let reconstruct = |reason| DecompileError::Reconstruct(version, reason);
    let code = module.code(module.root).map_err(reconstruct)?;
    if module.text(code.name).map_err(reconstruct)? != "<module>" {
        return Err(reconstruct("expected module code object".into()));
    }
    let filename = module.text(code.filename).map_err(reconstruct)?;
    let mut mismatch = None;
    for source in source::recover(&module).map_err(reconstruct)? {
        // The header does not record the optimization level.
        for level in [
            OptimizationLevel::Zero,
            OptimizationLevel::One,
            OptimizationLevel::Two,
        ] {
            let options = CompileOptions {
                python_version: version,
                optimization_level: level,
                ..CompileOptions::default()
            };
            if let Ok(compiled) = compile_with_options(&source, filename, &options) {
                let check = compiled.to_timestamp_pyc(0, 0);
                let candidate =
                    read(&check).map_err(|message| DecompileError::Verify(version, message))?;
                match verify::compare(&module, &candidate) {
                    Ok(()) => return Ok(source),
                    Err(reason) => {
                        mismatch.get_or_insert(reason);
                    }
                }
            }
        }
    }
    Err(DecompileError::Verify(
        version,
        mismatch.unwrap_or_else(|| "<module>: reconstructed source could not be compiled".into()),
    ))
}
