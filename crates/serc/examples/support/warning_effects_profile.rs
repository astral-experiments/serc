//! Partial warning effects admitted only for an already qualified standard caller.

use std::collections::{BTreeMap, BTreeSet};

use serc::{
    CompilationWarningEffect, CompileError, CompileOptions, CompiledModule, PreparedCompilation,
    PythonVersion,
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::CompilerInput;

// Exact artifacts admitted by independent source and loader-selection review.
fn admitted_proof(version: PythonVersion) -> (&'static [u8], &'static str) {
    match version {
        PythonVersion::Py312 => (
            include_bytes!("../../scripts/target_metadata/cpython_3_12_14_warning_effects.json"),
            "068a26d0467ed8d7a085b6478f5fd26904b41aa73c0e09e8de7d0aa6d503dc3f",
        ),
        PythonVersion::Py313 => (
            include_bytes!("../../scripts/target_metadata/cpython_3_13_15_warning_effects.json"),
            "827e54b19cb4dc7033548ec9be103ef68a9583f02d881cb37738f04e5af9871e",
        ),
        PythonVersion::Py314 => (
            include_bytes!("../../scripts/target_metadata/cpython_3_14_5_warning_effects.json"),
            "1dd195bf8ea05f03ab7ce1b21efc0535e55a5693f8af5e253a1b5eb546c80f56",
        ),
        PythonVersion::Py315 => (
            include_bytes!("../../scripts/target_metadata/cpython_3_15_0rc2_warning_effects.json"),
            "cf1e1ddd1253d988da4e1127d70d5d8dc0123646231d44ff64061ce3992ade1b",
        ),
    }
}

/// Guaranteed post-warning membership; unlisted additions remain unknown.
pub(super) struct WarningContext {
    python_version: PythonVersion,
    retained_strings: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceProof {
    schema_version: u32,
    kind: String,
    target: Target,
    initial_source_proof_sha256: String,
    runtime_admission_sha256: String,
    generator_sha256: String,
    code_tree_helper_sha256: String,
    runtime_verifier_sha256: String,
    compiler_executable_sha256: String,
    input_sha256: BTreeMap<String, String>,
    loader_selection: LoaderSelection,
    warning_categories: Vec<String>,
    semantics: String,
    retained_strings: Vec<String>,
    modules: Vec<Module>,
    conditions: Conditions,
    coverage: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    implementation: String,
    version: String,
    tag: String,
    commit: String,
    magic_hex: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Module {
    name: String,
    source: String,
    source_sha256: String,
    runtime_path: String,
    immortal_code_identifiers: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoaderSelection {
    frozen_source_sha256: BTreeMap<String, String>,
    builtin_table_runtime_path: String,
    builtin_table_sha256: String,
    builtin_names: Vec<String>,
    source_modules: Vec<String>,
    initially_loaded_modules: Vec<String>,
    search_path_contract: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Conditions {
    initial_caller_and_runtime_qualified: bool,
    empty_warnoptions: bool,
    bytes_warning: u8,
    native_default_warning_dispatch: bool,
    unmodified_import_hooks_and_modules: bool,
    source_only_warning_module_selection: bool,
}

impl WarningContext {
    /// Bind the source proof to the actual independently qualified invocation.
    ///
    /// Raw captures and inherited interpreters cannot call this path. In particular,
    /// visible warning output does not establish the required source import facts.
    pub(super) fn for_qualified_caller(
        initial_proof_sha256: &str,
        runtime_admission_sha256: &str,
        python_version: PythonVersion,
    ) -> Result<Self, String> {
        let (bytes, admitted_sha256) = admitted_proof(python_version);
        Self::from_proof(
            bytes,
            admitted_sha256,
            initial_proof_sha256,
            runtime_admission_sha256,
            python_version,
        )
    }

    fn from_proof(
        bytes: &[u8],
        admitted_sha256: &str,
        initial_proof_sha256: &str,
        runtime_admission_sha256: &str,
        python_version: PythonVersion,
    ) -> Result<Self, String> {
        require(
            digest(bytes) == admitted_sha256,
            "warning source proof has not been independently admitted",
        )?;
        let proof: SourceProof =
            serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        let target = python_version.target();
        require(
            proof.schema_version == 1
                && proof.kind == "standard-caller-warning-source-proof"
                && proof.semantics == "partial-warning-retained-positives",
            "unsupported warning proof schema or effect semantics",
        )?;
        require(
            proof.target.implementation == target.implementation
                && proof.target.version == target.version_string()
                && proof.target.tag == target.tag
                && proof.target.commit == target.commit
                && proof.target.magic_hex == target.magic_hex(),
            "warning source proof belongs to a different target",
        )?;
        require(
            proof.initial_source_proof_sha256 == initial_proof_sha256
                && proof.runtime_admission_sha256 == runtime_admission_sha256,
            "warning proof is not bound to this qualified initial state and runtime",
        )?;
        require(
            proof.generator_sha256
                == digest(include_bytes!(
                    "../../scripts/generate_cpython_warning_effects.py"
                ))
                && proof.code_tree_helper_sha256
                    == digest(include_bytes!(
                        "../../scripts/generate_cpython_bootstrap.py"
                    )),
            "warning proof generator or code-tree helper changed",
        )?;
        require(
            proof.runtime_verifier_sha256
                == digest(include_bytes!(
                    "../../scripts/generate_cpython_caller_variants.py"
                ))
                && proof.compiler_executable_sha256.len() == 64
                && proof
                    .compiler_executable_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "warning facts were not generated under the reviewed runtime qualifier",
        )?;
        let conditions = proof.conditions;
        require(
            conditions.initial_caller_and_runtime_qualified
                && conditions.empty_warnoptions
                && conditions.bytes_warning == 0
                && conditions.native_default_warning_dispatch
                && conditions.unmodified_import_hooks_and_modules
                && conditions.source_only_warning_module_selection
                && proof.warning_categories == ["SyntaxWarning"],
            "unsupported warning caller conditions",
        )?;
        let expected_modules: &[&str] = match python_version {
            PythonVersion::Py312 | PythonVersion::Py313 => &["warnings"],
            PythonVersion::Py314 | PythonVersion::Py315 => &["warnings", "_py_warnings"],
        };
        let selection = proof.loader_selection;
        require(
            selection.frozen_source_sha256.len() == 2
                && ["Python/frozen.c", "Tools/build/freeze_modules.py"]
                    .iter()
                    .all(|path| {
                        selection.frozen_source_sha256.contains_key(*path)
                            && selection.frozen_source_sha256.get(*path)
                                == proof.input_sha256.get(*path)
                    })
                && selection.source_modules == expected_modules
                && selection.initially_loaded_modules
                    == if matches!(python_version, PythonVersion::Py312 | PythonVersion::Py313) {
                        vec!["warnings"]
                    } else {
                        Vec::<&str>::new()
                    }
                && sorted_unique(&selection.builtin_names)
                && expected_modules.iter().all(|name| {
                    !selection
                        .builtin_names
                        .iter()
                        .any(|builtin| builtin == name)
                })
                && selection
                    .builtin_table_runtime_path
                    .starts_with(&format!("lib/python{python_version}/config-"))
                && selection.builtin_table_runtime_path.ends_with("/config.c")
                && selection.builtin_table_sha256.len() == 64
                && selection.search_path_contract
                    == "the bound initial proof's pristine importer state and qualified runtime search path",
            "warning loader selection is not covered by the qualified source/runtime proof",
        )?;
        require(
            proof.modules.len() == expected_modules.len(),
            "warning module set changed",
        )?;
        let mut union = BTreeSet::new();
        for (module, name) in proof.modules.iter().zip(expected_modules) {
            let source = format!("Lib/{name}.py");
            let runtime_path = format!("lib/python{python_version}/{name}.py");
            require(
                module.name == *name
                    && module.source == source
                    && module.runtime_path == runtime_path
                    && proof.input_sha256.get(&source) == Some(&module.source_sha256)
                    && sorted_unique(&module.immortal_code_identifiers),
                "warning module source or identifier proof changed",
            )?;
            union.extend(module.immortal_code_identifiers.iter().cloned());
        }
        require(
            sorted_unique(&proof.retained_strings)
                && proof.retained_strings.iter().eq(union.iter())
                && proof.coverage.is_object(),
            "warning retained positives differ from the proved code identifiers",
        )?;
        Ok(Self {
            python_version,
            retained_strings: proof.retained_strings,
        })
    }

    /// Replay explicit partial effects against this exact immutable input and plan.
    pub(super) fn compile(
        &self,
        input: &CompilerInput,
        filename: &str,
        options: &CompileOptions,
    ) -> Result<CompiledModule, CompileError> {
        if options.python_version != self.python_version {
            return Err(CompileError::InvalidCompilationEffects(
                "warning context belongs to a different target".to_string(),
            ));
        }
        let prepared = match input {
            CompilerInput::Bytes(source) => PreparedCompilation::bytes(source, filename, options)?,
            CompilerInput::Text(source) => PreparedCompilation::text(source, filename, options)?,
        };
        let mut effects = prepared.effects();
        for warning in prepared.warnings() {
            if warning.category() != "SyntaxWarning" {
                return Err(CompileError::InvalidCompilationEffects(
                    "warning category is outside the qualified caller proof".to_string(),
                ));
            }
            effects.record(
                warning.ordinal(),
                CompilationWarningEffect::PartialRetainedStrings(self.retained_strings.clone()),
            )?;
        }
        prepared.finish(&effects)
    }
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_string())
    }
}

fn sorted_unique(values: &[String]) -> bool {
    !values.is_empty() && values.windows(2).all(|pair| pair[0] < pair[1])
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::{SourceProof, WarningContext, admitted_proof, digest};
    const SOURCE_PROOF: &[u8] =
        include_bytes!("../../scripts/target_metadata/cpython_3_14_5_warning_effects.json");
    use crate::{CompilerInput, CompilerOutcome, compile_input_with_warning_context};
    use serc::{CompileError, CompileOptions, PythonVersion};

    #[test]
    fn all_pinned_warning_proofs_have_consistent_partial_source_facts() {
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let (bytes, _) = admitted_proof(version);
            let proof: SourceProof = serde_json::from_slice(bytes).unwrap();
            WarningContext::for_qualified_caller(
                &proof.initial_source_proof_sha256,
                &proof.runtime_admission_sha256,
                version,
            )
            .unwrap();
        }
    }

    #[test]
    fn warning_effects_require_the_bound_source_proof_and_runtime() {
        let proof: SourceProof = serde_json::from_slice(SOURCE_PROOF).unwrap();
        assert!(
            WarningContext::for_qualified_caller("foreign", "foreign", PythonVersion::Py314)
                .is_err()
        );
        for (initial, runtime, version) in [
            (
                "foreign",
                proof.runtime_admission_sha256.as_str(),
                PythonVersion::Py314,
            ),
            (
                proof.initial_source_proof_sha256.as_str(),
                "foreign",
                PythonVersion::Py314,
            ),
            (
                proof.initial_source_proof_sha256.as_str(),
                proof.runtime_admission_sha256.as_str(),
                PythonVersion::Py313,
            ),
        ] {
            assert!(
                WarningContext::from_proof(
                    SOURCE_PROOF,
                    &digest(SOURCE_PROOF),
                    initial,
                    runtime,
                    version
                )
                .is_err()
            );
        }
        let mut tampered = SOURCE_PROOF.to_vec();
        tampered.push(b' ');
        assert!(
            WarningContext::from_proof(
                &tampered,
                &digest(SOURCE_PROOF),
                &proof.initial_source_proof_sha256,
                &proof.runtime_admission_sha256,
                PythonVersion::Py314,
            )
            .is_err()
        );
    }

    #[test]
    fn partial_warning_certificate_resolves_known_members_and_rejects_unknown_identity() {
        let proof: SourceProof = serde_json::from_slice(SOURCE_PROOF).unwrap();
        let context = WarningContext::for_qualified_caller(
            &proof.initial_source_proof_sha256,
            &proof.runtime_admission_sha256,
            PythonVersion::Py314,
        )
        .unwrap();
        let source = "if 1 is 1: pass\ndef first(x): return x in {'catch_warnings', 'warning_effect_marker'}\ndef second(x): return x in {'catch_warnings', 'warning_effect_marker'}\n";
        let options = CompileOptions::default();
        assert!(matches!(
            serc::compile_with_options(source, "warning.py", &options),
            Err(CompileError::UnresolvedInternState { .. })
        ));
        let compiled = context
            .compile(
                &CompilerInput::Text(source.to_string()),
                "warning.py",
                &options,
            )
            .unwrap();
        let input = CompilerInput::Text(source.to_string());
        assert!(matches!(
            compile_input_with_warning_context(&input, "warning.py", &options, None, None),
            CompilerOutcome::UnresolvedInternState(_)
        ));
        let retried = compile_input_with_warning_context(
            &input,
            "warning.py",
            &options,
            None,
            Some(&context),
        );
        assert!(matches!(retried, CompilerOutcome::Compiled(bytes) if bytes == compiled.marshal()));
        let unknown = source.replace("catch_warnings", "another_warning_effect_marker");
        assert!(matches!(
            compile_input_with_warning_context(
                &CompilerInput::Text(unknown.clone()),
                "warning.py",
                &options,
                None,
                Some(&context)
            ),
            CompilerOutcome::UnresolvedInternState(_)
        ));
        assert!(matches!(
            compile_input_with_warning_context(
                &CompilerInput::Text("if".to_string()),
                "warning.py",
                &options,
                None,
                Some(&context)
            ),
            CompilerOutcome::Rejected(_)
        ));
        assert!(matches!(
            context.compile(&CompilerInput::Text(unknown), "warning.py", &options),
            Err(CompileError::UnresolvedInternState { .. })
        ));
    }
}
