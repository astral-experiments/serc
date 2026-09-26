//! Partial native Unicode effects admitted for an already qualified standard caller.

use std::collections::{BTreeMap, BTreeSet};

use serc::{
    CompilationEffect, CompilationEffects, CompilationEvent, CompileError, PreparedCompilation,
    PythonVersion,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const REQUIRED_SOURCES: [&str; 20] = [
    "Include/internal/pycore_ucnhash.h",
    "Modules/unicodedata.c",
    "Modules/clinic/unicodedata.c.h",
    "Modules/unicodedata_db.h",
    "Modules/unicodename_db.h",
    "Objects/capsule.c",
    "Objects/typeobject.c",
    "Objects/moduleobject.c",
    "Objects/object.c",
    "Objects/descrobject.c",
    "Objects/dictobject.c",
    "Objects/unicodeobject.c",
    "Python/modsupport.c",
    "Python/import.c",
    "Python/frozen.c",
    "Parser/pegen.c",
    "Parser/string_parser.c",
    "Lib/importlib/_bootstrap.py",
    "Lib/importlib/_bootstrap_external.py",
    "Tools/build/freeze_modules.py",
];

// These are admission pins, independently reviewed after source and runtime qualification.
fn admitted_proof(version: PythonVersion) -> (&'static [u8], &'static str) {
    match version {
        PythonVersion::Py312 => (
            include_bytes!("../../scripts/target_metadata/cpython_3_12_14_unicode_effects.json"),
            "ee6885df2e1e53e0a2a9dd90d0a5781651950ca66149ba264fe5634609a78a8d",
        ),
        PythonVersion::Py313 => (
            include_bytes!("../../scripts/target_metadata/cpython_3_13_15_unicode_effects.json"),
            "129b0ccff8d9d4a587a28e2d1ab1e7e9ea2c295459609998d9c94ed6a643d923",
        ),
        PythonVersion::Py314 => (
            include_bytes!("../../scripts/target_metadata/cpython_3_14_5_unicode_effects.json"),
            "e8919848bc0dfd2a2376144d85fd80a5a868264fdfb2ef3cdbfc8058f9e5cb5a",
        ),
        PythonVersion::Py315 => (
            include_bytes!("../../scripts/target_metadata/cpython_3_15_0rc2_unicode_effects.json"),
            "365864d6105596ca780e42cfebdddbbfceb580819ff0f2affd315fbe95346644",
        ),
    }
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
    target_helper_sha256: String,
    input_sha256: BTreeMap<String, String>,
    semantics: String,
    retained_strings: Vec<String>,
    loader_selection: LoaderSelection,
    conditions: Conditions,
    coverage: Coverage,
    unchanged_domain: Option<UnchangedDomain>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnchangedDomain {
    domain: String,
    semantics: String,
    initial_dotted_sha256: String,
    excluded_prefixes: Vec<String>,
}

// The outer admitted artifact binds the companion's complete bytes, including its
// all-core/frozen source inventory. Deserialize only the additional consistency checks.
#[derive(Deserialize)]
struct DottedProof {
    target: Target,
    initial_profile_sha256: String,
    runtime_admission_sha256: String,
    generator_sha256: String,
    lexical_helper_sha256: String,
    domain: String,
    possible_names: Vec<String>,
}

const EXCLUDED_PREFIXES: [&str; 4] = ["GraphemeBreakIterator.", "Segment.", "UCD.", "unicodedata."];

fn dotted_proof(version: PythonVersion) -> Option<&'static [u8]> {
    match version {
        PythonVersion::Py312 => None,
        PythonVersion::Py313 => Some(include_bytes!(
            "../../scripts/target_metadata/cpython_3_13_15_caller_dotted.json"
        )),
        PythonVersion::Py314 => Some(include_bytes!(
            "../../scripts/target_metadata/cpython_3_14_5_caller_dotted.json"
        )),
        PythonVersion::Py315 => Some(include_bytes!(
            "../../scripts/target_metadata/cpython_3_15_0rc2_caller_dotted.json"
        )),
    }
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
struct LoaderSelection {
    kind: String,
    config_path: String,
    config_sha256: String,
    carrier_path: String,
    carrier_sha256: String,
    libpython_path: String,
    libpython_sha256: String,
    executable_path: String,
    executable_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Conditions {
    qualified_initial_caller_and_runtime: bool,
    native_normalizer_and_name_capsule: bool,
    unmodified_import_hooks_and_modules: bool,
    pristine_source_only_runtime: bool,
    standard_source_codecs: bool,
    retained_owners_preserved: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Coverage {
    method_keys: Vec<String>,
    module_keys: Vec<String>,
    owners: String,
    other_additions: String,
    temporary_normalization_results: String,
    lookup_caches: String,
}

/// Partial native effects, optionally preserving a closed domain of incoming absence.
pub(super) struct UnicodeContext {
    python_version: PythonVersion,
    retained_strings: Vec<String>,
    possible_dotted_names: Option<BTreeSet<String>>,
}

impl UnicodeContext {
    /// Admit effects only after the same initial caller and owned runtime were qualified.
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
            "Unicode source proof has not been independently admitted",
        )?;
        let proof: SourceProof =
            serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        let target = python_version.target();
        require(
            proof.schema_version == 1
                && proof.kind == "standard-caller-unicode-parsing-source-proof"
                && proof.semantics == "partial-parser-retained-positives"
                && proof.target.implementation == target.implementation
                && proof.target.version == target.version_string()
                && proof.target.tag == target.tag
                && proof.target.commit == target.commit
                && proof.target.magic_hex == target.magic_hex(),
            "unsupported Unicode source proof schema or target",
        )?;
        require(
            proof.initial_source_proof_sha256 == initial_proof_sha256
                && proof.runtime_admission_sha256 == runtime_admission_sha256,
            "Unicode proof is not bound to this qualified initial state and runtime",
        )?;
        require(
            proof.generator_sha256
                == digest(include_bytes!(
                    "../../scripts/generate_cpython_unicode_effects.py"
                ))
                && proof.target_helper_sha256
                    == digest(include_bytes!("../../scripts/generate_cpython_target.py")),
            "Unicode source proof generator changed",
        )?;
        let conditions = proof.conditions;
        require(
            conditions.qualified_initial_caller_and_runtime
                && conditions.native_normalizer_and_name_capsule
                && conditions.unmodified_import_hooks_and_modules
                && conditions.pristine_source_only_runtime
                && conditions.standard_source_codecs
                && conditions.retained_owners_preserved,
            "unsupported Unicode caller conditions",
        )?;
        let selection = proof.loader_selection;
        let libpython = format!("lib/libpython{python_version}.so.1.0");
        require(
            selection
                .config_path
                .starts_with(&format!("lib/python{python_version}/config-"))
                && selection.config_path.ends_with("/config.c")
                && valid_relative_path(&selection.config_path)
                && valid_relative_path(&selection.carrier_path)
                && selection.executable_path == format!("bin/python{python_version}")
                && selection.libpython_path == libpython
                && [
                    &selection.config_sha256,
                    &selection.carrier_sha256,
                    &selection.libpython_sha256,
                    &selection.executable_sha256,
                ]
                .into_iter()
                .all(|hash| valid_hash(hash))
                && match selection.kind.as_str() {
                    "builtin" => {
                        selection.carrier_path == libpython
                            && selection.carrier_sha256 == selection.libpython_sha256
                    }
                    "extension" => {
                        selection.carrier_path.starts_with(&format!(
                            "lib/python{python_version}/lib-dynload/unicodedata."
                        )) && std::path::Path::new(&selection.carrier_path)
                            .extension()
                            .is_some_and(|extension| extension == "so")
                    }
                    _ => false,
                },
            "Unicode native selection differs from the admitted runtime contract",
        )?;
        require(
            proof.input_sha256.len() == REQUIRED_SOURCES.len()
                && REQUIRED_SOURCES
                    .iter()
                    .all(|path| proof.input_sha256.contains_key(*path))
                && proof
                    .input_sha256
                    .iter()
                    .all(|(path, hash)| valid_relative_path(path) && valid_hash(hash)),
            "Unicode source proof omits required ownership or lookup inputs",
        )?;
        let coverage = proof.coverage;
        let mut retained = coverage.method_keys;
        retained.extend(coverage.module_keys);
        retained.sort();
        require(
            !retained.is_empty()
                && retained.windows(2).all(|pair| pair[0] < pair[1])
                && proof.retained_strings == retained
                && coverage.owners == "native module dictionary and UCD method/member descriptors"
                && coverage.other_additions == "unknown"
                && coverage.temporary_normalization_results == "not claimed"
                && coverage.lookup_caches
                    == "parser-local normalizer and interpreter-wide name capsule",
            "Unicode effects do not preserve partial retained-owner semantics",
        )?;
        let possible_dotted_names = match (proof.unchanged_domain, dotted_proof(python_version)) {
            (None, None) => None,
            (Some(domain), Some(bytes)) => {
                require(
                    domain.domain == "ascii-dotted-identifiers-and-locals"
                        && domain.semantics == "incoming-absence-preservation-only"
                        && domain.excluded_prefixes == EXCLUDED_PREFIXES
                        && domain.initial_dotted_sha256 == digest(bytes),
                    "Unicode preservation domain differs from the admitted dotted closure",
                )?;
                let dotted: DottedProof =
                    serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
                require(
                    dotted.target.implementation == target.implementation
                        && dotted.target.version == target.version_string()
                        && dotted.target.tag == target.tag
                        && dotted.target.commit == target.commit
                        && dotted.target.magic_hex == target.magic_hex()
                        && dotted.initial_profile_sha256 == initial_proof_sha256
                        && dotted.runtime_admission_sha256 == runtime_admission_sha256
                        && dotted.domain == domain.domain
                        && dotted.generator_sha256
                            == digest(include_bytes!(
                                "../../scripts/generate_cpython_caller_dotted.py"
                            ))
                        && dotted.lexical_helper_sha256
                            == digest(include_bytes!("../../scripts/caller_dotted_metadata.py"))
                        && dotted
                            .possible_names
                            .windows(2)
                            .all(|pair| pair[0] < pair[1])
                        && dotted
                            .possible_names
                            .iter()
                            .all(|value| super::standard_caller_profile::ascii_dotted_name(value)),
                    "Unicode dotted closure belongs to a different caller, runtime or source",
                )?;
                Some(dotted.possible_names.into_iter().collect())
            }
            _ => return Err("Unicode preservation domain does not match the target".to_string()),
        };
        Ok(Self {
            python_version,
            retained_strings: proof.retained_strings,
            possible_dotted_names,
        })
    }

    /// Exclude only external native additions; target parser/compiler ownership is separate.
    fn preserves_absence(&self, value: &str) -> bool {
        self.possible_dotted_names.as_ref().is_some_and(|possible| {
            super::standard_caller_profile::ascii_dotted_name(value)
                && !possible.contains(value)
                && !EXCLUDED_PREFIXES
                    .iter()
                    .any(|prefix| value.starts_with(prefix))
        })
    }

    /// Fill only Unicode event slots; warning ordinals keep their separate mapping.
    pub(super) fn record(
        &self,
        prepared: &PreparedCompilation,
        effects: &mut CompilationEffects,
    ) -> Result<(), CompileError> {
        if prepared.target() != self.python_version.target() {
            return Err(CompileError::InvalidCompilationEffects(
                "Unicode context belongs to a different target".to_string(),
            ));
        }
        for (index, event) in prepared.events().iter().enumerate() {
            if matches!(event, CompilationEvent::UnicodeParsing(_)) {
                effects.record_event(
                    index,
                    CompilationEffect::PartialRetainedStringsWithUnchanged {
                        retained_strings: self.retained_strings.clone(),
                        unchanged_strings: prepared
                            .intern_state_candidates()
                            .iter()
                            .filter(|value| self.preserves_absence(value))
                            .cloned()
                            .collect(),
                    },
                )?;
            }
        }
        Ok(())
    }
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_string())
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_relative_path(value: &str) -> bool {
    !value.starts_with('/')
        && value
            .split('/')
            .all(|part| !matches!(part, "" | "." | ".."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serc::CompileOptions;
    use serde_json::{Value, json};

    fn validate(bytes: &[u8], version: PythonVersion) -> Result<UnicodeContext, String> {
        let proof: SourceProof = serde_json::from_slice(bytes).unwrap();
        UnicodeContext::from_proof(
            bytes,
            &digest(bytes),
            &proof.initial_source_proof_sha256,
            &proof.runtime_admission_sha256,
            version,
        )
    }

    #[test]
    fn native_unicode_proofs_bind_each_pinned_target() {
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let (bytes, _) = admitted_proof(version);
            validate(bytes, version).unwrap();
            assert!(UnicodeContext::for_qualified_caller("foreign", "foreign", version).is_err());
        }
    }

    #[test]
    fn unicode_proof_rejects_foreign_contexts_and_modified_artifacts() {
        let (bytes, _) = admitted_proof(PythonVersion::Py314);
        let proof: SourceProof = serde_json::from_slice(bytes).unwrap();
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
                UnicodeContext::from_proof(bytes, &digest(bytes), initial, runtime, version)
                    .is_err()
            );
        }
        let mut modified = bytes.to_vec();
        modified.push(b' ');
        assert!(
            UnicodeContext::from_proof(
                &modified,
                &digest(bytes),
                &proof.initial_source_proof_sha256,
                &proof.runtime_admission_sha256,
                PythonVersion::Py314
            )
            .is_err()
        );
    }

    #[test]
    fn unicode_proof_requires_native_bindings_and_partial_ownership() {
        let (bytes, _) = admitted_proof(PythonVersion::Py314);
        for (pointer, replacement) in [
            ("/generator_sha256", json!("0".repeat(64))),
            ("/target_helper_sha256", json!("0".repeat(64))),
            ("/loader_selection/carrier_path", json!("lib/other.so")),
            (
                "/loader_selection/config_path",
                json!("lib/python3.14/config-../../../../config.c"),
            ),
            ("/loader_selection/libpython_sha256", json!("0".repeat(64))),
            (
                "/conditions/native_normalizer_and_name_capsule",
                json!(false),
            ),
            ("/coverage/other_additions", json!("absent")),
            ("/semantics", json!("complete-parser-retained-state")),
            (
                "/unchanged_domain/initial_dotted_sha256",
                json!("0".repeat(64)),
            ),
            ("/unchanged_domain/domain", json!("all-strings")),
            ("/unchanged_domain/semantics", json!("new-absence")),
            ("/unchanged_domain/excluded_prefixes", json!([])),
        ] {
            let mut value: Value = serde_json::from_slice(bytes).unwrap();
            *value.pointer_mut(pointer).unwrap() = replacement;
            // Rehashing exercises semantic checks independently of the outer admission pin.
            assert!(
                validate(&serde_json::to_vec(&value).unwrap(), PythonVersion::Py314).is_err(),
                "{pointer}"
            );
        }
        for source_key in [
            "Modules/unicodedata.c",
            "Objects/typeobject.c",
            "Python/frozen.c",
        ] {
            let mut value: Value = serde_json::from_slice(bytes).unwrap();
            value["input_sha256"]
                .as_object_mut()
                .unwrap()
                .remove(source_key);
            assert!(validate(&serde_json::to_vec(&value).unwrap(), PythonVersion::Py314).is_err());
        }
        let (extension, _) = admitted_proof(PythonVersion::Py313);
        let mut value: Value = serde_json::from_slice(extension).unwrap();
        value["loader_selection"]["kind"] = json!("builtin");
        assert!(validate(&serde_json::to_vec(&value).unwrap(), PythonVersion::Py313).is_err());
    }

    #[test]
    fn unicode_absence_preservation_stays_within_its_closed_domain() {
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let (bytes, _) = admitted_proof(version);
            let context = validate(bytes, version).unwrap();
            assert_eq!(
                context.preserves_absence("Owner.Inner"),
                version != PythonVersion::Py312
            );
            for value in [
                "bare",
                "Owner.é",
                "/path/Owner.Inner",
                "Owner..Inner",
                "unicodedata.UCD",
                "UCD.normalize",
                "Segment.start",
                "GraphemeBreakIterator.next",
            ] {
                assert!(!context.preserves_absence(value), "{version}: {value}");
            }
            if let Some(possible) = &context.possible_dotted_names {
                assert!(
                    possible
                        .iter()
                        .all(|value| !context.preserves_absence(value))
                );
            }
        }
    }

    #[test]
    fn unicode_preservation_neither_establishes_absence_nor_bypasses_later_warnings() {
        use crate::{CompilerInput, CompilerOutcome, compile_input_with_warning_context};

        let (bytes, _) = admitted_proof(PythonVersion::Py314);
        let proof: SourceProof = serde_json::from_slice(bytes).unwrap();
        let context = crate::warning_effects_profile::WarningContext::for_qualified_caller(
            &proof.initial_source_proof_sha256,
            &proof.runtime_admission_sha256,
            PythonVersion::Py314,
        )
        .unwrap();
        for known_absence in [false, true] {
            for warning in ["", "if 1 is 1: pass\n"] {
                let input = CompilerInput::Text(format!(
                    "\u{212a} = 0\n{warning}class Owner:\n    class Inner: pass\n"
                ));
                let options = CompileOptions {
                    initial_intern_state: serc::InitialInternState::from_partial_with_absence(
                        [] as [&str; 0],
                        move |_, value| known_absence && value == "Owner.Inner",
                    ),
                    ..CompileOptions::default()
                };
                let result = compile_input_with_warning_context(
                    &input,
                    "unicode.py",
                    &options,
                    None,
                    Some(&context),
                );
                if known_absence && warning.is_empty() {
                    assert!(matches!(result, CompilerOutcome::Compiled(_)));
                } else {
                    assert!(matches!(result, CompilerOutcome::UnresolvedInternState(_)));
                }
            }
        }
    }

    #[test]
    fn unicode_context_records_only_its_target_and_keeps_plan_ownership() {
        let (bytes, _) = admitted_proof(PythonVersion::Py314);
        let context = validate(bytes, PythonVersion::Py314).unwrap();
        let foreign = PreparedCompilation::text(
            "pass",
            "foreign.py",
            &CompileOptions {
                python_version: PythonVersion::Py313,
                ..CompileOptions::default()
            },
        )
        .unwrap();
        assert!(context.record(&foreign, &mut foreign.effects()).is_err());
        let plan =
            PreparedCompilation::text("\u{212a} = 1", "unicode.py", &CompileOptions::default())
                .unwrap();
        let mut effects = plan.effects();
        context.record(&plan, &mut effects).unwrap();
        assert!(context.record(&plan, &mut effects).is_err());
        let other =
            PreparedCompilation::text("\u{212a} = 1", "other.py", &CompileOptions::default())
                .unwrap();
        assert!(matches!(
            other.finish(&effects),
            Err(CompileError::InvalidCompilationEffects(_))
        ));
        plan.finish(&effects).unwrap();
    }

    #[test]
    fn qualified_adapter_replays_unicode_and_later_warning_effects() {
        use crate::{CompilerInput, CompilerOutcome, compile_input_with_warning_context};

        let (bytes, _) = admitted_proof(PythonVersion::Py314);
        let proof: SourceProof = serde_json::from_slice(bytes).unwrap();
        let context = crate::warning_effects_profile::WarningContext::for_qualified_caller(
            &proof.initial_source_proof_sha256,
            &proof.runtime_admission_sha256,
            PythonVersion::Py314,
        )
        .unwrap();
        let consumers = "def first(value):\n    return value in {'normalize', 'is_normalized'}, 1\ndef second(value):\n    'second docstring'\n    return value in {'normalize', 'is_normalized'}, 2\n";
        for warning in ["", "if 1 is 1: pass\n"] {
            let source = CompilerInput::Text(format!("\u{212a} = 1\n{warning}{consumers}"));
            let options = CompileOptions::default();
            assert!(matches!(
                compile_input_with_warning_context(&source, "unicode.py", &options, None, None),
                CompilerOutcome::UnresolvedInternState(_)
            ));
            assert!(matches!(
                compile_input_with_warning_context(
                    &source,
                    "unicode.py",
                    &options,
                    None,
                    Some(&context)
                ),
                CompilerOutcome::Compiled(_)
            ));
        }
    }
}
