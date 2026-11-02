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
    fn exception_warnings_require_a_mismatch_at_every_optimization_level() {
        let cache = |source: &str| {
            crate::compile(source, "handlers.py")
                .unwrap()
                .to_timestamp_pyc(0, 0)
        };
        let original = "def f():\n    try: work()\n    except KeyError: raise\n    return 1\n";
        let input = cache(original);
        // A different return value still prevents whole-module verification.
        let same_table = cache(&original.replace("return 1", "return 2"));
        let different_table = cache("def f():\n    work()\n    return 2\n");
        for matching_level in [None, Some(crate::OptimizationLevel::Two)] {
            let output = serc_decompile::decompile_pyc_with_mode(
                &input,
                DecompileMode::BestEffort,
                |_, _, _, level| {
                    Some(if Some(level) == matching_level {
                        same_table.clone()
                    } else {
                        different_table.clone()
                    })
                },
            )
            .unwrap();
            assert!(output.verification_error.is_some());
            assert_eq!(
                output
                    .source
                    .contains("def f():  # serc: exception table and normal control flow differ"),
                matching_level.is_none(),
            );
        }
    }

    #[test]
    fn normal_flow_warnings_require_different_edges_at_every_level() {
        let original = "def route(a, b):\n    if a and b: work()\n    return a + 1\n";
        let cache = |source: &str| {
            crate::compile(source, "route.py")
                .unwrap()
                .to_timestamp_pyc(0, 0)
        };
        let input = cache(original);
        // Changing which successor short-circuits keeps the block count.
        let different_flow = cache(&original.replace("a and b", "a or b"));
        let same_flow = cache(&original.replace("a + 1", "a - 1"));
        for matching_level in [None, Some(crate::OptimizationLevel::Two)] {
            let output = serc_decompile::decompile_pyc_with_mode(
                &input,
                DecompileMode::BestEffort,
                |_, _, _, level| {
                    Some(if Some(level) == matching_level {
                        same_flow.clone()
                    } else {
                        different_flow.clone()
                    })
                },
            )
            .unwrap();
            assert!(output.verification_error.is_some());
            assert_eq!(
                output
                    .source
                    .contains("def route(a, b):  # serc: normal control flow differs"),
                matching_level.is_none()
            );
            assert_eq!(
                output
                    .source
                    .contains("# serc: original blocks (normal control flow differs) in route "),
                matching_level.is_none()
            );
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
