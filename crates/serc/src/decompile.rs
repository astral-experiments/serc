//! Verified source recovery using serc's compiler.
pub use serc_decompile::{DecompileError, DecompileMode, DecompileOutput};

use crate::{CompileOptions, compile_with_options};

/// Reconstruct Python source from a cache without executing it. Output that
/// could not be verified carries a best-effort warning.
pub fn decompile_pyc(bytes: &[u8]) -> Result<String, DecompileError> {
    Ok(decompile_pyc_with_mode(bytes, DecompileMode::BestEffort)?.source)
}

/// Recover source with structured verification status. Select
/// [`DecompileMode::Strict`] to require an exact executable-field match.
pub fn decompile_pyc_with_mode(
    bytes: &[u8],
    mode: DecompileMode,
) -> Result<DecompileOutput, DecompileError> {
    serc_decompile::decompile_pyc_with_mode(
        bytes,
        mode,
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
        let result = serc_decompile::decompile_pyc_with_mode(
            &original,
            DecompileMode::Strict,
            |_, _, _, _| Some(candidate.clone()),
        );
        assert!(
            matches!(result, Err(DecompileError::Verify(PythonVersion::Py314, reason)) if reason.contains("3.15"))
        );
    }
}
