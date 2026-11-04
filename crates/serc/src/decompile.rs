//! Recover Python source and verify it with serc's compiler.
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
    fn repeated_names_keep_their_own_bodies_when_a_handler_is_unplaced() {
        let cache = crate::compile(
            "try:\n    risky()\nexcept ValueError:\n    def same(): return 1\nexcept KeyError:\n    def same(): return 2\nafter = 42\n",
            "handlers.py",
        )
        .unwrap()
        .to_timestamp_pyc(0, 0);
        let source = decompile_pyc(&cache).unwrap();
        assert_eq!(source.matches("def same():").count(), 2);
        assert_eq!(source.matches("return 1").count(), 1);
        assert_eq!(source.matches("return 2").count(), 1);
        if let Some((_, detached)) = source.split_once("# serc: detached function ") {
            assert!(detached.starts_with("same (original line 6)\n"));
            assert!(detached.contains("#     return 2"));
        }
    }

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
