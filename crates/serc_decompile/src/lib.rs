//! Reconstruct Python source from the instruction stream stored in a CPython cache file.
mod flow;
mod source;
mod verify;

use serc_bytecode::marshal::reader::read;
use serc_bytecode::{OptimizationLevel, PythonVersion};
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
/// nested scope rejects the entire module. The compiler callback receives the
/// source, original filename, target, and optimization level. It returns a cache
/// for that candidate or `None` when the candidate cannot be compiled.
pub fn decompile_pyc(
    bytes: &[u8],
    mut compile: impl FnMut(&str, &str, PythonVersion, OptimizationLevel) -> Option<Vec<u8>>,
) -> Result<String, DecompileError> {
    let module = read(bytes).map_err(DecompileError::Read)?;
    let version = module.version;
    let reconstruct = |reason| DecompileError::Reconstruct(version, reason);
    let code = module.code(module.root).map_err(reconstruct)?;
    if module.text(code.name).map_err(reconstruct)? != "<module>" {
        return Err(reconstruct("expected module code object".into()));
    }
    let filename = module.text(code.filename).map_err(reconstruct)?;
    let mut mismatch = None;
    // A NOP can be a user statement or compiler-generated tracing machinery.
    // Try explicit pass statements only after the simpler spelling fails.
    for preserve_nops in [false, true] {
        let (sources, saw_nop) = match source::recover(&module, preserve_nops) {
            Ok(result) => result,
            Err(_) if preserve_nops => break,
            Err(message) => return Err(reconstruct(message)),
        };
        for source in sources {
            // The header does not record the optimization level.
            for level in [
                OptimizationLevel::Zero,
                OptimizationLevel::One,
                OptimizationLevel::Two,
            ] {
                if let Some(check) = compile(&source, filename, version, level) {
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
        if !saw_nop {
            break;
        }
    }
    Err(DecompileError::Verify(
        version,
        mismatch.unwrap_or_else(|| "<module>: reconstructed source could not be compiled".into()),
    ))
}
