//! Source-proved caller state for an explicitly qualified, immutable runtime.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serc::{InitialInternState, PythonVersion};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
#[cfg(unix)]
use walkdir::WalkDir;

use super::initial_intern_profile::LoadedProfile;

// Only independently reviewed complete source proofs may supply an absence rule.
// A proof document cannot authorize itself by setting `closure_complete`.
const REVIEWED_PROOFS: &[&str] = &[];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceProof {
    schema_version: u32,
    kind: String,
    target: Target,
    supported_compiler_input_modes: Vec<String>,
    oracle: Oracle,
    platform: String,
    architecture: String,
    gil_enabled: bool,
    generator_sha256: String,
    input_sha256: BTreeMap<String, String>,
    startup_profile_sha256: String,
    state: State,
    runtime_admissions: Vec<Artifact>,
    coverage: Coverage,
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
struct Oracle {
    script_sha256: String,
    arguments: Vec<String>,
    source_input: String,
    compile_mode: String,
    dont_inherit: bool,
    optimize: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    retained_strings: Vec<String>,
    retained_runtime_paths: Vec<String>,
    possible_ascii_words: Vec<String>,
    absence_domain: String,
    closure_complete: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    path: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Coverage {
    module_loads: Vec<Value>,
    retained_owners: Vec<Value>,
    intern_sinks: Vec<Value>,
    dynamic_name_recipes: Vec<Value>,
    unresolved: Vec<Value>,
}

/// An owned runtime whose immutable contents and environment were qualified.
pub(super) struct QualifiedOracle {
    runtime: tempfile::TempDir,
    executable: PathBuf,
    environment: BTreeMap<String, String>,
}

impl QualifiedOracle {
    /// Construct the unchanged oracle invocation under the admitted environment.
    pub(super) fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command.env_clear().envs(&self.environment);
        command
    }
}

impl Drop for QualifiedOracle {
    fn drop(&mut self) {
        // TempDir removes the copy after directory write permissions are restored.
        // The supplied interpreter installation is never modified.
        restore_directories(self.runtime.path());
    }
}

fn restore_directories(root: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
            if entry.file_type().is_dir() {
                let _ =
                    fs_err::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o700));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = root;
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedRuntime {
    prefix: PathBuf,
    executable: PathBuf,
    environment: BTreeMap<String, String>,
}

/// Load an independently admitted source proof and qualify one supported build.
pub(super) fn load(
    path: &Path,
    bytes: &[u8],
    python: &str,
    version: PythonVersion,
    source_bytes: bool,
    oracle: &str,
) -> Result<LoadedProfile, String> {
    let proof_hash = digest(bytes);
    require(
        REVIEWED_PROOFS.contains(&proof_hash.as_str()),
        "standard caller source proof has not been independently admitted",
    )?;
    let proof: SourceProof = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    proof.validate(version, source_bytes, oracle)?;
    let directory = path
        .parent()
        .ok_or("source proof has no containing directory")?;
    let preparer =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/prepare_cpython_caller.py");
    let preparer_hash = digest(&fs_err::read(&preparer).map_err(|error| error.to_string())?);
    let mut errors = Vec::new();
    for artifact in &proof.runtime_admissions {
        let admission_path = directory.join(relative_path(&artifact.path)?);
        let admission_bytes = fs_err::read(&admission_path).map_err(|error| error.to_string())?;
        require(
            digest(&admission_bytes) == artifact.sha256,
            "runtime admission artifact hash differs",
        )?;
        let admission: Value =
            serde_json::from_slice(&admission_bytes).map_err(|error| error.to_string())?;
        require(
            admission["preparer_sha256"].as_str() == Some(&preparer_hash),
            "runtime admission does not bind the reviewed preparer",
        )?;
        match prepare_runtime(python, &preparer, &admission_path, &admission) {
            Ok(qualified) => {
                let mut retained = proof.state.retained_strings.clone();
                for path in &proof.state.retained_runtime_paths {
                    retained.push(
                        qualified
                            .runtime
                            .path()
                            .join(relative_path(path)?)
                            .to_str()
                            .ok_or("runtime prefix is not UTF-8")?
                            .to_owned(),
                    );
                }
                let retained_entries = retained.len();
                let possible: BTreeSet<_> = proof.state.possible_ascii_words.into_iter().collect();
                let possible_entries = possible.len();
                let state = InitialInternState::from_partial_with_absence(
                    retained,
                    move |target, value| {
                        target == version && ascii_word(value) && !possible.contains(value)
                    },
                );
                return Ok(LoadedProfile {
                    state,
                    snapshot_sha256: proof_hash,
                    description: format!(
                        "{retained_entries} guaranteed retained entries; {possible_entries} possible ASCII words"
                    ),
                    qualified_oracle: Some(qualified),
                });
            }
            Err(error) => errors.push(error),
        }
    }
    Err(format!(
        "no reviewed runtime admission matched: {}",
        errors.join("; ")
    ))
}

fn prepare_runtime(
    python: &str,
    preparer: &Path,
    admission_path: &Path,
    admission: &Value,
) -> Result<QualifiedOracle, String> {
    let runtime = tempfile::Builder::new()
        .prefix("serc-caller-")
        .tempdir()
        .map_err(|error| error.to_string())?;
    let result = (|| {
        let output = Command::new(python)
            .args(["-B", "-I", "-S"])
            .arg(preparer)
            .arg(admission_path)
            .arg(runtime.path())
            .output()
            .map_err(|error| error.to_string())?;
        require(
            output.status.success(),
            &format!(
                "runtime qualification failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )?;
        let prepared: PreparedRuntime =
            serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
        let executable = runtime.path().join(relative_path(
            admission["executable"]
                .as_str()
                .ok_or("runtime executable missing")?,
        )?);
        let mut environment: BTreeMap<String, String> =
            serde_json::from_value(admission["environment"].clone())
                .map_err(|error| error.to_string())?;
        if let Some(loader) = admission["loader_library_path"].as_str() {
            environment.insert(
                "LD_LIBRARY_PATH".to_owned(),
                runtime
                    .path()
                    .join(relative_path(loader)?)
                    .to_str()
                    .ok_or("runtime prefix is not UTF-8")?
                    .to_owned(),
            );
        }
        require(
            prepared.prefix == runtime.path()
                && prepared.executable == executable
                && prepared.environment == environment,
            "prepared runtime response differs from admission",
        )?;
        Ok((executable, environment))
    })();
    match result {
        Ok((executable, environment)) => Ok(QualifiedOracle {
            runtime,
            executable,
            environment,
        }),
        Err(error) => {
            restore_directories(runtime.path());
            Err(error)
        }
    }
}

impl SourceProof {
    fn validate(
        &self,
        version: PythonVersion,
        source_bytes: bool,
        oracle: &str,
    ) -> Result<(), String> {
        let target = version.target();
        require(
            self.schema_version == 1 && self.kind == "standard-caller-source-proof",
            "unsupported standard caller proof schema",
        )?;
        require(
            self.target.implementation == target.implementation
                && self.target.version == target.version_string()
                && self.target.tag == target.tag
                && self.target.commit == target.commit
                && self.target.magic_hex == target.magic_hex(),
            "source proof target differs from compiler target",
        )?;
        let mode = if source_bytes {
            "source-bytes"
        } else {
            "utf8-text"
        };
        require(
            self.supported_compiler_input_modes
                .iter()
                .any(|value| value == mode),
            "source proof does not support the compiler input mode",
        )?;
        require(
            self.oracle.script_sha256 == digest(oracle.as_bytes())
                && self.oracle.arguments == ["-I", "-S", "-c"]
                && self.oracle.source_input == "bytes"
                && self.oracle.compile_mode == "exec"
                && self.oracle.dont_inherit
                && self.oracle.optimize == 0,
            "source proof differs from the unchanged oracle",
        )?;
        require(
            cfg!(target_os = "linux")
                && cfg!(target_arch = "x86_64")
                && self.platform == "linux"
                && self.architecture == "x86_64"
                && self.gil_enabled,
            "standard caller proof requires Linux x86_64 with the GIL",
        )?;
        require(
            self.state.closure_complete && self.coverage.unresolved.is_empty(),
            "source proof has unresolved closure obligations",
        )?;
        require(
            !self.coverage.module_loads.is_empty()
                && !self.coverage.retained_owners.is_empty()
                && !self.coverage.intern_sinks.is_empty()
                && !self.coverage.dynamic_name_recipes.is_empty(),
            "source proof coverage is incomplete",
        )?;
        require(
            !self.input_sha256.is_empty()
                && valid_hash(&self.generator_sha256)
                && valid_hash(&self.startup_profile_sha256)
                && self.input_sha256.values().all(|value| valid_hash(value)),
            "source proof provenance hashes are invalid",
        )?;
        require(
            self.state.absence_domain == "nonempty-ascii-alnum-underscore",
            "source proof absence domain is unsupported",
        )?;
        for values in [
            &self.state.retained_strings,
            &self.state.retained_runtime_paths,
            &self.state.possible_ascii_words,
        ] {
            require(
                values.windows(2).all(|pair| pair[0] < pair[1]),
                "source proof state must be sorted and unique",
            )?;
        }
        let possible: BTreeSet<_> = self
            .state
            .possible_ascii_words
            .iter()
            .map(String::as_str)
            .collect();
        require(
            possible.iter().all(|value| ascii_word(value))
                && self
                    .state
                    .retained_strings
                    .iter()
                    .filter(|value| ascii_word(value))
                    .all(|value| possible.contains(value.as_str())),
            "source proof possible set does not cover its retained ASCII words",
        )?;
        require(
            !self.runtime_admissions.is_empty(),
            "source proof has no reviewed runtime admission",
        )?;
        Ok(())
    }
}

fn ascii_word(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn relative_path(value: &str) -> Result<&Path, String> {
    let path = Path::new(value);
    require(
        !value.is_empty()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
            && !value.contains('\\'),
        "invalid artifact-relative path",
    )?;
    Ok(path)
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claimed_completion_cannot_admit_an_unreviewed_source_proof() {
        let bytes = br#"{"kind":"standard-caller-source-proof","state":{"closure_complete":true}}"#;
        let error = load(
            Path::new("unreviewed.json"),
            bytes,
            "interpreter-must-not-run",
            PythonVersion::Py314,
            true,
            super::super::ORACLE,
        )
        .err()
        .expect("unreviewed proof must fail before interpreter invocation");
        assert!(error.contains("has not been independently admitted"));
    }

    #[cfg(unix)]
    #[test]
    fn dropping_a_qualified_runtime_removes_its_read_only_copy() {
        use std::os::unix::fs::PermissionsExt;
        let runtime = tempfile::tempdir().unwrap();
        let path = runtime.path().to_path_buf();
        let library = path.join("lib");
        fs_err::create_dir(&library).unwrap();
        fs_err::write(library.join("module.py"), "pass\n").unwrap();
        fs_err::set_permissions(&library, std::fs::Permissions::from_mode(0o555)).unwrap();
        fs_err::set_permissions(&path, std::fs::Permissions::from_mode(0o555)).unwrap();
        let qualified = QualifiedOracle {
            runtime,
            executable: path.join("python"),
            environment: BTreeMap::new(),
        };
        drop(qualified);
        assert!(!path.exists());
    }
}
