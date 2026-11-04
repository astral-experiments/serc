//! Reconstruct Python source from the instruction stream stored in a CPython cache file.
mod flow;
mod instructions;
mod source;
mod verify;

use crate::marshal::reader::read;
use crate::{CompileOptions, OptimizationLevel, compile_with_options};
use std::fmt;

/// A cache file is invalid, unsupported, or could not be faithfully reconstructed.
#[derive(Debug)]
pub struct DecompileError(String);

impl fmt::Display for DecompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for DecompileError {}

/// Recover an entire module as Python source without executing the input.
/// The cache magic selects one of serc's pinned targets.
pub fn decompile_pyc(bytes: &[u8]) -> Result<String, DecompileError> {
    let module = read(bytes).map_err(DecompileError)?;
    let version = module.version;
    let recover = || -> Result<String, String> {
        let code = module.code(module.root)?;
        if module.text(code.name)? != "<module>" {
            return Err("expected module code object".into());
        }
        let filename = module.text(code.filename)?;
        // A pyc header does not record the optimization level. Accept the
        // candidate only if one of the supported levels reproduces its code.
        for source in source::recover(&module)? {
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
                    if verify::equivalent(&module, &read(&check)?) {
                        return Ok(source);
                    }
                }
            }
        }
        Err("<module>: reconstructed source did not reproduce the input code".into())
    };
    recover().map_err(|message| DecompileError(format!("{version}: {message}")))
}
