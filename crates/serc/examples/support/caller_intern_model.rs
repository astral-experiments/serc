//! Opt-in replay of a reviewed caller's retained POSIX path components.
//!
//! The sealed profile remains native provenance. The effective compiler state is
//! an independent pre-path baseline, a source-derived compiler phase, and the
//! current filename's retained components. No state accumulates between files.

use std::collections::BTreeSet;
use std::path::Path;

use serc::{CompileOptions, InitialInternState, PythonVersion};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::native_output_cache::{Artifact, artifact, digest, regular_bytes};

const MODE: &str = "pathlib-posix-source-bytes-v1";
const IMPLEMENTATION: &str = "cpython-pathlib-posix-parts-v1";
const ORACLE_SHA256: &str = "6812e016e7e871700001ad9f8f832c5998d553bc33c84afb1745dbd664a965fb";
const BASELINE_SHA256: &str = "af650692baf658821c1bd969fc0016d3e94300fb74755bcb1058486994c3b669";
const FIXED_PHASE_SHA256: &str = "1bd4edaf3f6b04c859224ecc819c2ec5d09e59a60207c2085e28c7dc5fcc8ccc";
const PATHLIB_SHA256: &str = "10ba48ae8063cfe7589436041d0d4628c05f427c7709cf8e75e486f8930cc32c";
const POSIXPATH_SHA256: &str = "03825681086638649a43480954f7f6a16b4da3bd41ece956864ae96e4f795cd9";
const SEALED_PROFILE_SHA256: &str =
    "63c0a5a8e13abd26c756b9790081ed0d98f1c9d0960ae933b565847c38e3b51d";
const EXECUTABLE_SHA256: &str = "febd623a8152b648d90174369fb7eb121be1ed653fa0fa5d72e3b512cce8aa8e";
const RUNTIME_INPUTS_SHA256: &str =
    "5c5010b0998ee3e501130a05b28c7fca9c38574302d9b2b9a56195271c915c9a";
const BOUNDARY: &str = "sys_intern:entry:first-call-from-pinned-pathlib-_parse_path";
const POST_BOUNDARY: &str = "Py_CompileStringObject:entry";

pub(super) struct LoadedModel {
    state: InitialInternState,
    pub(super) evidence: Value,
    pub(super) inputs: Vec<Artifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    schema_version: u32,
    mode: String,
    target: Value,
    compiler_input_mode: String,
    oracle_sha256: String,
    sealed_profile: Artifact,
    executable: Artifact,
    runtime_inputs: Artifact,
    baseline: Artifact,
    fixed_phase: Artifact,
    model: Model,
    observations: Vec<Artifact>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Model {
    implementation: String,
    rust_sha256: String,
    pathlib: Artifact,
    posixpath: Artifact,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    schema_version: u32,
    mode: String,
    target: Value,
    baseline_sha256: String,
    fixed_phase_sha256: String,
    sealed_profile_sha256: String,
    executable_sha256: String,
    runtime_inputs_sha256: String,
    oracle_sha256: String,
    boundary: String,
    post_boundary: String,
    invariant: bool,
    controls: Vec<Artifact>,
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InternEntry {
    pool: String,
    state: u8,
    codepoints: Vec<u32>,
}

/// Called only after the unchanged sealed-profile validator has accepted its
/// complete runtime closure. This loader binds that identity without treating
/// its neutral-path snapshot as the effective compiler state.
pub(super) fn load(
    path: &Path,
    sealed_profile_path: &Path,
    python_version: PythonVersion,
    source_bytes: bool,
    oracle: &str,
) -> Result<LoadedModel, String> {
    let descriptor_bytes = regular_bytes(path).map_err(|error| error.to_string())?;
    let descriptor: Descriptor = read_json(&descriptor_bytes)?;
    descriptor.validate_contract(python_version, source_bytes, oracle)?;
    require(
        descriptor.sealed_profile.path == sealed_profile_path,
        "caller model names a different sealed profile",
    )?;
    let mut inputs = vec![artifact(path, &descriptor_bytes)];
    let profile: Value = read_json(&read_artifact(&descriptor.sealed_profile, &mut inputs)?)?;
    require(
        profile["target"] == descriptor.target
            && profile["compiler_input_mode"] == descriptor.compiler_input_mode
            && profile["oracle"]["script_sha256"] == descriptor.oracle_sha256,
        "caller model target or oracle differs from the sealed profile",
    )?;
    for (name, item) in [
        ("executable", &descriptor.executable),
        ("runtime_inputs", &descriptor.runtime_inputs),
    ] {
        require(
            profile[name]["path"].as_str() == item.path.to_str()
                && profile[name]["sha256"] == item.sha256,
            "caller model executable or runtime identity differs from the sealed profile",
        )?;
        read_artifact(item, &mut inputs)?;
    }
    read_artifact(&descriptor.model.pathlib, &mut inputs)?;
    read_artifact(&descriptor.model.posixpath, &mut inputs)?;
    let baseline = read_entries(&read_artifact(&descriptor.baseline, &mut inputs)?)?;
    require(
        baseline.len() == 5494
            && baseline.contains("")
            && (0..=255).all(|point| baseline.contains(&char::from(point).to_string())),
        "caller baseline lacks the reviewed complete intern table",
    )?;
    let phase = read_entries(&read_artifact(&descriptor.fixed_phase, &mut inputs)?)?;
    require(phase.len() == 86, "caller fixed compiler phase differs")?;
    for observation in &descriptor.observations {
        let observation: Observation = read_json(&read_artifact(observation, &mut inputs)?)?;
        observation.validate(&descriptor)?;
        for control in &observation.controls {
            let control: Value = read_json(&read_artifact(control, &mut inputs)?)?;
            let raw: Vec<Artifact> = serde_json::from_value(control["raw_artifacts"].clone())
                .map_err(|error| error.to_string())?;
            require(
                !raw.is_empty(),
                "caller control raw observation evidence is missing",
            )?;
            for item in &raw {
                read_artifact(item, &mut inputs)?;
            }
        }
        for item in &observation.artifacts {
            read_artifact(item, &mut inputs)?;
        }
    }
    let state = InitialInternState::from_complete(baseline.into_iter().chain(phase));
    let evidence = json!({
        "mode": descriptor.mode,
        "descriptor": artifact(path, &descriptor_bytes),
        "target": descriptor.target,
        "oracle_sha256": descriptor.oracle_sha256,
        "sealed_profile": descriptor.sealed_profile,
        "baseline": descriptor.baseline,
        "fixed_phase": descriptor.fixed_phase,
        "model": descriptor.model,
        "observations": descriptor.observations,
        "effective_state": "complete-pre-path-baseline-plus-fixed-phase-plus-retained-path-components",
        "per_compilation_reset": true,
        "filename_contract": "original-literal-UTF8-POSIX-path-already-canonical-for-pathlib",
    });
    Ok(LoadedModel {
        state,
        evidence,
        inputs,
    })
}

impl Descriptor {
    fn validate_contract(
        &self,
        python_version: PythonVersion,
        source_bytes: bool,
        oracle: &str,
    ) -> Result<(), String> {
        require(
            cfg!(target_os = "linux"),
            "caller model requires the reviewed Linux POSIX platform",
        )?;
        require(
            python_version == PythonVersion::Py312,
            "caller model has no reviewed invariant baseline for this target (only Python 3.12 is admitted)",
        )?;
        let target = python_version.target();
        require(
            target.commit == "2abcf904b8dac8c999d2b3aac76681abb333798a"
                && self.target
                    == json!({
                        "implementation": target.implementation,
                        "version": target.version_string(),
                        "tag": target.tag,
                        "commit": target.commit,
                        "magic_hex": target.magic_hex(),
                    }),
            "caller model CPython target differs from its reviewed implementation",
        )?;
        require(
            self.schema_version == 1 && self.mode == MODE,
            "unsupported caller model schema or mode",
        )?;
        require(
            source_bytes && self.compiler_input_mode == "source-bytes",
            "caller model requires source-bytes input",
        )?;
        require(
            self.oracle_sha256 == ORACLE_SHA256 && digest(oracle.as_bytes()) == ORACLE_SHA256,
            "caller model requires the unchanged reviewed oracle",
        )?;
        require(
            self.sealed_profile.sha256 == SEALED_PROFILE_SHA256
                && self.executable.sha256 == EXECUTABLE_SHA256
                && self.runtime_inputs.sha256 == RUNTIME_INPUTS_SHA256,
            "caller model runtime or sealed profile has not been independently reviewed",
        )?;
        require(
            self.baseline.sha256 == BASELINE_SHA256
                && self.fixed_phase.sha256 == FIXED_PHASE_SHA256,
            "unknown caller baseline or fixed compiler phase identity",
        )?;
        require(
            self.model.implementation == IMPLEMENTATION
                && self.model.rust_sha256 == digest(include_bytes!("caller_intern_model.rs"))
                && self.model.pathlib.sha256 == PATHLIB_SHA256
                && self.model.posixpath.sha256 == POSIXPATH_SHA256,
            "unknown caller parsing implementation identity",
        )?;
        require(
            !self.observations.is_empty(),
            "caller observation evidence is missing",
        )
    }
}

impl Observation {
    fn validate(&self, descriptor: &Descriptor) -> Result<(), String> {
        require(
            self.schema_version == 1
                && self.mode == MODE
                && self.target == descriptor.target
                && self.baseline_sha256 == descriptor.baseline.sha256
                && self.fixed_phase_sha256 == descriptor.fixed_phase.sha256
                && self.sealed_profile_sha256 == descriptor.sealed_profile.sha256
                && self.executable_sha256 == descriptor.executable.sha256
                && self.runtime_inputs_sha256 == descriptor.runtime_inputs.sha256
                && self.oracle_sha256 == descriptor.oracle_sha256
                && self.boundary == BOUNDARY
                && self.post_boundary == POST_BOUNDARY
                && self.invariant
                && !self.controls.is_empty()
                && !self.artifacts.is_empty(),
            "caller observation does not bind the reviewed baseline, phase, and runtime boundaries",
        )
    }
}

impl LoadedModel {
    pub(super) fn options_for(&self, filename: &str, options: &CompileOptions) -> CompileOptions {
        CompileOptions {
            python_version: options.python_version,
            initial_intern_state: self.state.with_additional_strings(retained_parts(filename)),
        }
    }
}

/// Keep the original compiler filename. The oracle formats `str(path)`, so a
/// literal argument that would change during that operation is unsupported by
/// this mode and rejects the invocation before any files are compared.
pub(super) fn validate_filename(path: &Path) -> Result<(), String> {
    let filename = path
        .to_str()
        .ok_or("caller model requires UTF-8 filenames")?;
    require(
        !filename.contains('\0'),
        "caller model rejects NUL filenames",
    )?;
    require(
        canonical_filename(filename) == filename,
        "pathlib-posix-source-bytes-v1 requires an original filename already canonical for pathlib; filename remapping is forbidden",
    )
}

// Both pinned 3.12 pathlib.py and 3.13 pathlib/_local.py use
// [sys.intern(str(x)) for x in rel.split(sep) if x and x != '.'].
// POSIX splitroot has no drive and removes only leading '/' characters from rel;
// filtering empty components therefore excludes either '/' or '//' root without
// interning it. '..', backslashes, and colon-containing components remain literal.
fn retained_parts(filename: &str) -> impl Iterator<Item = &str> {
    filename
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
}

fn canonical_filename(filename: &str) -> String {
    let root = if filename.starts_with("//") && !filename.starts_with("///") {
        "//"
    } else if filename.starts_with('/') {
        "/"
    } else {
        ""
    };
    let tail = retained_parts(filename).collect::<Vec<_>>().join("/");
    if root.is_empty() && tail.is_empty() {
        ".".to_string()
    } else {
        format!("{root}{tail}")
    }
}

fn read_entries(bytes: &[u8]) -> Result<BTreeSet<String>, String> {
    let entries: Vec<InternEntry> = read_json(bytes)?;
    let mut strings = BTreeSet::new();
    for entry in entries {
        require(
            matches!(
                (entry.pool.as_str(), entry.state),
                ("static", 3) | ("dynamic", 1 | 2)
            ),
            "invalid caller intern pool/state",
        )?;
        let value = entry
            .codepoints
            .into_iter()
            .map(|point| {
                char::from_u32(point).ok_or("caller state contains a non-scalar codepoint")
            })
            .collect::<Result<String, _>>()?;
        require(strings.insert(value), "duplicate caller intern value")?;
    }
    Ok(strings)
}

fn read_artifact(item: &Artifact, inputs: &mut Vec<Artifact>) -> Result<Vec<u8>, String> {
    let bytes = item.read().map_err(|error| error.to_string())?;
    inputs.push(item.clone());
    Ok(bytes)
}

fn read_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|error| error.to_string())
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> Descriptor {
        let target = PythonVersion::Py312.target();
        let placeholder =
            json!({"path": "/evidence/artifact", "bytes": 0, "sha256": "placeholder"});
        let mut value = json!({
            "schema_version": 1, "mode": MODE,
            "target": {"implementation": target.implementation, "version": target.version_string(),
                "tag": target.tag, "commit": target.commit, "magic_hex": target.magic_hex()},
            "compiler_input_mode": "source-bytes", "oracle_sha256": ORACLE_SHA256,
            "sealed_profile": placeholder, "executable": placeholder, "runtime_inputs": placeholder,
            "baseline": placeholder, "fixed_phase": placeholder,
            "model": {"implementation": IMPLEMENTATION,
                "rust_sha256": digest(include_bytes!("caller_intern_model.rs")),
                "pathlib": placeholder, "posixpath": placeholder},
            "observations": [placeholder],
        });
        value["sealed_profile"]["sha256"] = json!(SEALED_PROFILE_SHA256);
        value["executable"]["sha256"] = json!(EXECUTABLE_SHA256);
        value["runtime_inputs"]["sha256"] = json!(RUNTIME_INPUTS_SHA256);
        value["baseline"]["sha256"] = json!(BASELINE_SHA256);
        value["fixed_phase"]["sha256"] = json!(FIXED_PHASE_SHA256);
        value["model"]["pathlib"]["sha256"] = json!(PATHLIB_SHA256);
        value["model"]["posixpath"]["sha256"] = json!(POSIXPATH_SHA256);
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn pinned_posix_parser_retains_components_but_not_drive_or_root() {
        for (filename, expected, canonical) in [
            ("", vec![], "."),
            (
                "./alpha//alpha/../file.py/",
                vec!["alpha", "alpha", "..", "file.py"],
                "alpha/alpha/../file.py",
            ),
            (
                "//server/share/./file.py",
                vec!["server", "share", "file.py"],
                "//server/share/file.py",
            ),
            (
                "///alpha//file.py",
                vec!["alpha", "file.py"],
                "/alpha/file.py",
            ),
            (
                "C:\\alpha/file.py",
                vec!["C:\\alpha", "file.py"],
                "C:\\alpha/file.py",
            ),
            (
                "../../file.py",
                vec!["..", "..", "file.py"],
                "../../file.py",
            ),
            ("//", vec![], "//"),
        ] {
            assert_eq!(retained_parts(filename).collect::<Vec<_>>(), expected);
            assert_eq!(canonical_filename(filename), canonical);
        }
    }

    #[test]
    fn filename_validation_preserves_the_literal_compiler_filename() {
        for filename in [
            "target/corpus/file.py",
            "//alpha/file.py",
            "../alpha/file.py",
        ] {
            assert!(validate_filename(Path::new(filename)).is_ok());
        }
        for filename in [
            "",
            "./alpha.py",
            "alpha//file.py",
            "alpha/./file.py",
            "///alpha.py",
            "alpha.py/",
        ] {
            assert!(validate_filename(Path::new(filename)).is_err());
        }
        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;
            assert!(validate_filename(Path::new(OsStr::from_bytes(b"bad\xff.py"))).is_err());
        }
    }

    #[test]
    fn filename_states_reset_and_replace_the_neutral_profile() {
        let model = LoadedModel {
            state: InitialInternState::from_complete(["bootstrap_atom"]),
            evidence: Value::Null,
            inputs: Vec::new(),
        };
        let options = CompileOptions {
            python_version: PythonVersion::Py312,
            initial_intern_state: InitialInternState::new(["retained_path_atom"]),
        };
        let source =
            "f = lambda x: x in {'retained_path_atom', 'z'}\nx in {'retained_path_atom', 'z'}\n";
        let compile = |options: &CompileOptions| {
            serc::compile_with_options(source, "unchanged.py", options)
                .unwrap()
                .marshal()
        };
        let original = compile(&options);
        let baseline = CompileOptions {
            python_version: PythonVersion::Py312,
            initial_intern_state: model.state.clone(),
        };
        let first = model.options_for("retained_path_atom/file.py", &options);
        let second = model.options_for("other_directory/file.py", &options);
        assert_ne!(compile(&first), compile(&baseline));
        assert_eq!(compile(&second), compile(&baseline));
        assert_eq!(
            compile(&model.options_for("other_directory/file.py", &options)),
            compile(&second),
        );
        assert_eq!(compile(&options), original);
    }

    #[test]
    fn contract_rejects_unreviewed_target_baseline_or_implementation() {
        let oracle = super::super::ORACLE;
        assert_eq!(
            descriptor()
                .validate_contract(PythonVersion::Py312, true, oracle)
                .is_ok(),
            cfg!(target_os = "linux")
        );
        for target in [
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            assert!(
                descriptor()
                    .validate_contract(target, true, oracle)
                    .is_err()
            );
        }
        assert!(
            descriptor()
                .validate_contract(PythonVersion::Py312, false, oracle)
                .is_err()
        );
        assert!(
            descriptor()
                .validate_contract(PythonVersion::Py312, true, "changed oracle")
                .is_err()
        );
        let mut changed = descriptor();
        changed.baseline.sha256 = "unknown".to_string();
        assert!(
            changed
                .validate_contract(PythonVersion::Py312, true, oracle)
                .is_err()
        );
        let mut changed = descriptor();
        changed.model.rust_sha256 = "unknown".to_string();
        assert!(
            changed
                .validate_contract(PythonVersion::Py312, true, oracle)
                .is_err()
        );
        let mut changed = descriptor();
        changed.mode = "unknown".to_string();
        assert!(
            changed
                .validate_contract(PythonVersion::Py312, true, oracle)
                .is_err()
        );
    }
}
