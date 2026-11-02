//! Verified source recovery using serc's compiler.
pub use serc_decompile::DecompileError;

use crate::{CompileOptions, compile_with_options};

/// Recover an entire module as Python source without executing its bytecode.
/// The cache magic selects one of serc's pinned targets. Each nested scope
/// must reproduce its executable fields when recompiled.
pub fn decompile_pyc(bytes: &[u8]) -> Result<String, DecompileError> {
    serc_decompile::decompile_pyc(
        bytes,
        |source, filename, python_version, optimization_level| {
            let options = CompileOptions {
                python_version,
                optimization_level,
                ..CompileOptions::default()
            };
            compile_with_options(source, filename, &options)
                .ok()
                .map(|module| module.to_timestamp_pyc(0, 0))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PythonVersion;

    #[test]
    fn verification_rejects_a_cache_for_another_target() {
        let original = crate::compile("pass\n", "module.py")
            .unwrap()
            .to_timestamp_pyc(0, 0);
        let mut candidate = original.clone();
        candidate[..4].copy_from_slice(&PythonVersion::Py315.target().magic_number);
        let result = serc_decompile::decompile_pyc(&original, |_, _, _, _| Some(candidate.clone()));
        assert!(
            matches!(result, Err(DecompileError::Verify(PythonVersion::Py314, reason)) if reason.contains("3.15"))
        );
    }
}
