//! Reconstruct Python source from the instruction stream stored in a CPython cache file.
mod flow;
mod source;
mod verify;

use serc_bytecode::marshal::reader::{Module, read};
use serc_bytecode::{OptimizationLevel, PythonVersion};
use source::synthetic::Mode::{DeadSuites, LexicalBindings, LiveGuards, Ordinary};
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

/// Whether to return a marked reconstruction when whole-module verification fails.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DecompileMode {
    #[default]
    BestEffort,
    Strict,
}

/// Source and the reason it could not be verified, if any.
#[derive(Debug)]
pub struct DecompileOutput {
    pub source: String,
    pub verification_error: Option<DecompileError>,
}

/// Recover source without executing the input. A best-effort result carries an
/// inline warning. Use [`decompile_pyc_with_mode`] for structured status.
pub fn decompile_pyc(
    bytes: &[u8],
    compile: impl FnMut(&str, &str, PythonVersion, OptimizationLevel) -> Option<Vec<u8>>,
) -> Result<String, DecompileError> {
    Ok(decompile_pyc_with_mode(bytes, DecompileMode::BestEffort, compile)?.source)
}

/// The compiler callback receives the original filename, target and candidate
/// optimization level. Strict mode checks every nested code object. Best-effort
/// mode first attempts the same verification and marks any fallback in the source.
pub fn decompile_pyc_with_mode(
    bytes: &[u8],
    mode: DecompileMode,
    mut compile: impl FnMut(&str, &str, PythonVersion, OptimizationLevel) -> Option<Vec<u8>>,
) -> Result<DecompileOutput, DecompileError> {
    let module = read(bytes).map_err(DecompileError::Read)?;
    let mut fallback = None;
    let keep = (mode == DecompileMode::BestEffort).then_some(&mut fallback);
    match verified(&module, &mut compile, keep) {
        Ok(source) => Ok(DecompileOutput {
            source,
            verification_error: None,
        }),
        Err(error) if mode == DecompileMode::BestEffort => {
            let result = source::partial(&module, fallback)
                .map_err(|reason| DecompileError::Reconstruct(module.version, reason))?;
            let note = source::comment(&error.to_string());
            Ok(DecompileOutput {
                source: format!("# serc: best-effort decompilation\n\n# {note}\n{result}"),
                verification_error: Some(error),
            })
        }
        Err(error) => Err(error),
    }
}

fn verified(
    module: &Module<'_>,
    compile: &mut impl FnMut(&str, &str, PythonVersion, OptimizationLevel) -> Option<Vec<u8>>,
    mut fallback: Option<&mut Option<source::Candidate>>,
) -> Result<String, DecompileError> {
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
    let mut saw_nop = true;
    let mut hints: Option<source::synthetic::Hints> = None;
    for (mode, preserve_nops) in [Ordinary, DeadSuites, LiveGuards, LexicalBindings]
        .into_iter()
        .flat_map(|mode| [(mode, false), (mode, true)])
    {
        if (preserve_nops && !saw_nop) || (mode == LexicalBindings && mismatch.is_some()) {
            continue;
        }
        let stubs = if matches!(mode, DeadSuites | LiveGuards) {
            let Some(plan) = hints.as_ref().filter(|plan| !plan.is_empty()) else {
                continue;
            };
            Some(plan)
        } else {
            None
        };
        let (sources, has_nop, definitions) =
            match source::recover(module, preserve_nops, stubs, mode) {
                Ok(result) => result,
                Err(_) if preserve_nops || mode != Ordinary => continue,
                Err(message) => return Err(reconstruct(message)),
            };
        saw_nop = has_nop;
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
                    match verify::compare(module, &candidate) {
                        Ok(()) => return Ok(source),
                        Err(reason) => {
                            if hints.is_none() && candidate.version == version {
                                hints = Some(
                                    source::synthetic::differences(module, &candidate)
                                        .unwrap_or_default(),
                                );
                            }
                            if candidate.version == version
                                && let Some(slot) = fallback.as_deref_mut()
                                && slot.is_none()
                            {
                                *slot = Some(source::Candidate::new(
                                    source.clone(),
                                    definitions.clone(),
                                ));
                            }
                            mismatch.get_or_insert(reason);
                        }
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
