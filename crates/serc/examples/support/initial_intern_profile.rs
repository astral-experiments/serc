//! Validated, explicit process snapshots for the corpus comparator.
//!
//! Hashes bind declared artifacts and the local runtime, not an artifact's author.
//! A snapshot records one process's entries. Only the authenticated static pool
//! proves membership across fresh invocations; dynamic observations stay unknown
//! without a separately reviewed caller contract.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serc::{InitialInternState, PythonVersion};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

fn capture_boundary(python_version: PythonVersion) -> &'static str {
    match python_version {
        PythonVersion::Py312 | PythonVersion::Py313 | PythonVersion::Py314 => {
            "Py_CompileStringObject:entry-after-argument-and-AST-init-before-parser"
        }
        PythonVersion::Py315 => "_Py_CompileString:entry-after-argument-and-AST-init-before-parser",
    }
}

pub(super) struct LoadedProfile {
    pub(super) state: InitialInternState,
    pub(super) snapshot_sha256: String,
    pub(super) entries: usize,
    pub(super) retained_entries: usize,
}

struct ValidatedSnapshot {
    entries: usize,
    static_strings: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    schema_version: u32,
    target: Target,
    compiler_input_mode: InputMode,
    oracle: Oracle,
    executable: Artifact,
    snapshot: Artifact,
    capture: Artifact,
    observer: Artifact,
    layout: Artifact,
    headers: Artifact,
    runtime_inputs: Artifact,
    process_maps: Artifact,
    runtime_capture: Artifact,
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

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum InputMode {
    SourceBytes,
    Utf8Text,
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
    capture_boundary: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    path: PathBuf,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InternEntry {
    pool: Pool,
    state: u8,
    codepoints: Vec<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Pool {
    Static,
    Dynamic,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeInputs {
    runtime_prefix: PathBuf,
    files: Vec<RuntimeFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeFile {
    path: PathBuf,
    sha256: String,
    size: u64,
    symlink_target: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeCapture {
    #[serde(rename = "before_manifest_sha256")]
    before_manifest: String,
    #[serde(rename = "after_manifest_sha256")]
    after_manifest: String,
    #[serde(rename = "capture_sha256")]
    capture: String,
    #[serde(rename = "process_maps_sha256")]
    process_maps: String,
}

pub(super) fn load(
    path: &Path,
    python: &str,
    python_version: PythonVersion,
    source_bytes: bool,
    oracle: &str,
) -> Result<LoadedProfile, String> {
    let profile: Profile = read_json(&read(path)?)?;
    profile.validate_contract(python_version, source_bytes, oracle)?;
    let executable = profile.executable.read()?;
    let snapshot = profile.snapshot.read()?;
    let capture: Value = read_json(&profile.capture.read()?)?;
    profile.observer.read()?;
    profile.layout.read()?;
    let headers: Value = read_json(&profile.headers.read()?)?;
    let maps =
        String::from_utf8(profile.process_maps.read()?).map_err(|error| error.to_string())?;
    let runtime: RuntimeInputs = read_json(&profile.runtime_inputs.read()?)?;
    let relation: RuntimeCapture = read_json(&profile.runtime_capture.read()?)?;
    require(
        relation.before_manifest == profile.runtime_inputs.sha256
            && relation.after_manifest == profile.runtime_inputs.sha256
            && relation.capture == profile.capture.sha256
            && relation.process_maps == profile.process_maps.sha256,
        "runtime capture relation does not bind identical before/after manifests to this capture",
    )?;
    // This separate verification process does not change the authoritative oracle.
    let output = Command::new(python)
        .args([
            "-I",
            "-S",
            "-c",
            "import sys; print(sys.executable); print(sys.prefix)",
        ])
        .output()
        .map_err(|error| format!("cannot resolve interpreter: {error}"))?;
    require(output.status.success(), "interpreter resolution failed")?;
    let output = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
    let lines: Vec<_> = output.lines().collect();
    require(lines.len() == 2, "unexpected interpreter resolution output")?;
    require(
        Path::new(lines[0]) == profile.executable.path
            && Path::new(lines[1]) == runtime.runtime_prefix,
        "interpreter executable or runtime prefix differs from the capture",
    )?;
    require(!executable.is_empty(), "empty interpreter executable")?;
    runtime.validate()?;
    runtime.validate_mapped_files(&maps)?;
    profile.validate_headers(&headers)?;
    profile.validate_capture(&capture, oracle)?;
    let snapshot = validate_snapshot(read_json(&snapshot)?, &capture)?;
    Ok(LoadedProfile {
        retained_entries: snapshot.static_strings.len(),
        state: InitialInternState::from_partial(snapshot.static_strings),
        snapshot_sha256: profile.snapshot.sha256,
        entries: snapshot.entries,
    })
}

impl Profile {
    fn validate_contract(
        &self,
        python_version: PythonVersion,
        source_bytes: bool,
        oracle: &str,
    ) -> Result<(), String> {
        let target = python_version.target();
        require(
            self.schema_version == 1,
            "unsupported profile schema version",
        )?;
        require(
            self.target.implementation == target.implementation
                && self.target.version == target.version_string()
                && self.target.tag == target.tag
                && self.target.commit == target.commit
                && self.target.magic_hex == target.magic_hex(),
            "profile CPython target differs from the compiler target",
        )?;
        let mode = if source_bytes {
            InputMode::SourceBytes
        } else {
            InputMode::Utf8Text
        };
        require(
            self.compiler_input_mode == mode,
            "profile compiler input mode differs from CLI mode",
        )?;
        require(
            self.oracle.script_sha256 == digest(oracle.as_bytes())
                && self.oracle.arguments == ["-I", "-S", "-c"]
                && self.oracle.source_input == "bytes"
                && self.oracle.compile_mode == "exec"
                && self.oracle.dont_inherit
                && self.oracle.optimize == 0
                && self.oracle.capture_boundary == capture_boundary(python_version),
            "profile oracle script, invocation, or capture boundary differs",
        )
    }

    fn validate_headers(&self, headers: &Value) -> Result<(), String> {
        require(
            headers.get("executable_sha256").and_then(Value::as_str)
                == Some(&self.executable.sha256),
            "header provenance executable differs",
        )?;
        require(
            headers.get("layout_json_sha256").and_then(Value::as_str) == Some(&self.layout.sha256),
            "header provenance layout differs",
        )?;
        let directory = Path::new(
            headers
                .get("installed_headers")
                .and_then(Value::as_str)
                .ok_or("missing installed header directory")?,
        );
        let entries = headers
            .get("headers")
            .and_then(Value::as_object)
            .ok_or("missing header fingerprints")?;
        require(!entries.is_empty(), "empty header provenance")?;
        for (name, entry) in entries {
            let expected = entry
                .get("installed_sha256")
                .and_then(Value::as_str)
                .ok_or("missing installed header hash")?;
            require(
                digest(&read(&directory.join(name))?) == expected,
                "installed header changed",
            )?;
            if name != "pyconfig.h" {
                require(
                    entry.get("exact_tag_file_sha256").and_then(Value::as_str) == Some(expected)
                        && entry.get("matches_exact_tag") == Some(&Value::Bool(true)),
                    "header does not match the exact source tag",
                )?;
            }
        }
        Ok(())
    }

    fn validate_capture(&self, capture: &Value, oracle: &str) -> Result<(), String> {
        for (pointer, expected) in [
            ("/context", Value::from("native")),
            (
                "/binary_sha256",
                Value::from(self.executable.sha256.clone()),
            ),
            (
                "/script_sha256",
                Value::from(self.oracle.script_sha256.clone()),
            ),
            (
                "/canonical_sha256",
                Value::from(self.snapshot.sha256.clone()),
            ),
            (
                "/observer_sha256",
                Value::from(self.observer.sha256.clone()),
            ),
            ("/layout_sha256", Value::from(self.layout.sha256.clone())),
            (
                "/interpreter",
                Value::from(self.executable.path.to_string_lossy().into_owned()),
            ),
            ("/thread_count", Value::from(1)),
            ("/two_complete_reads_equal", Value::from(true)),
            ("/compile_function_memory_matches_elf", Value::from(true)),
            ("/utf8_excluded_values", Value::from(0)),
            ("/snapshot/empty_and_latin1_present", Value::from(true)),
        ] {
            require(
                capture.pointer(pointer) == Some(&expected),
                &format!("capture metadata mismatch at {pointer}"),
            )?;
        }
        let argv = capture
            .get("argv")
            .and_then(Value::as_array)
            .ok_or("capture argv missing")?;
        require(
            argv.len() == 6
                && argv[0].as_str() == self.executable.path.to_str()
                && argv[1] == "-I"
                && argv[2] == "-S"
                && argv[3] == "-c"
                && argv[4] == oracle,
            "capture argv differs from the exact oracle invocation",
        )?;
        let source_path = Path::new(argv[5].as_str().ok_or("capture source path missing")?);
        require(
            read(source_path)?.is_empty()
                && capture.get("source_sha256").and_then(Value::as_str) == Some(&digest(b""))
                && capture.get("compile_entry_hits") == Some(&serde_json::json!([argv[5]])),
            "capture must use a generic empty source and exactly one target compile entry",
        )?;
        Ok(())
    }
}

impl Artifact {
    fn read(&self) -> Result<Vec<u8>, String> {
        require(self.path.is_absolute(), "artifact paths must be absolute")?;
        let bytes = read(&self.path)?;
        require(
            digest(&bytes) == self.sha256,
            &format!("SHA-256 mismatch: {}", self.path.display()),
        )?;
        Ok(bytes)
    }
}

impl RuntimeInputs {
    fn validate_mapped_files(&self, maps: &str) -> Result<(), String> {
        let recorded: BTreeSet<_> = self.files.iter().map(|file| file.path.as_path()).collect();
        let mapped: BTreeSet<_> = maps
            .lines()
            .filter_map(|line| line.split_whitespace().nth(5))
            .filter(|name| name.starts_with('/'))
            .map(Path::new)
            .collect();
        require(
            !mapped.is_empty() && mapped.is_subset(&recorded),
            "runtime manifest omits captured mapped native files",
        )
    }

    fn validate(&self) -> Result<(), String> {
        require(
            self.runtime_prefix.is_absolute(),
            "runtime prefix must be absolute",
        )?;
        let mut recorded = BTreeSet::new();
        for file in &self.files {
            require(
                file.path.is_absolute() && recorded.insert(file.path.clone()),
                "runtime paths must be absolute and unique",
            )?;
            let target = fs_err::read_link(&file.path).ok();
            require(
                target == file.symlink_target,
                &format!("runtime symlink changed: {}", file.path.display()),
            )?;
            let bytes = read(&file.path)?;
            require(
                bytes.len() as u64 == file.size && digest(&bytes) == file.sha256,
                &format!("runtime content changed: {}", file.path.display()),
            )?;
        }
        let mut current = BTreeSet::new();
        for entry in WalkDir::new(&self.runtime_prefix).follow_links(false) {
            let entry = entry.map_err(|error| error.to_string())?;
            require(
                !(entry.file_type().is_symlink() && entry.path().is_dir()),
                "runtime directory symlinks are unsupported",
            )?;
            require(
                entry.file_type().is_dir() || entry.path().is_file(),
                "runtime manifest cannot cover a special file or dangling symlink",
            )?;
            if entry.path().is_file() {
                current.insert(entry.into_path());
            }
        }
        let declared = recorded
            .into_iter()
            .filter(|path| path.starts_with(&self.runtime_prefix))
            .collect();
        require(
            current == declared && !current.is_empty(),
            "runtime manifest does not cover the complete installed prefix",
        )
    }
}

fn validate_snapshot(
    entries: Vec<InternEntry>,
    capture: &Value,
) -> Result<ValidatedSnapshot, String> {
    let mut seen = BTreeSet::new();
    let mut static_strings = Vec::new();
    let mut states = BTreeMap::<u8, usize>::new();
    for entry in entries {
        require(
            matches!(
                (&entry.pool, entry.state),
                (Pool::Static, 3) | (Pool::Dynamic, 1 | 2)
            ),
            "invalid intern pool/state combination",
        )?;
        let value: String = entry
            .codepoints
            .into_iter()
            .map(|point| char::from_u32(point).ok_or("snapshot contains a non-scalar codepoint"))
            .collect::<Result<_, _>>()?;
        // Static canonical objects belong to this authenticated build. Dynamic
        // observations, including immortal objects, do not prove membership in
        // every new caller process or retention under a different input.
        if matches!(entry.pool, Pool::Static) {
            static_strings.push(value.clone());
        }
        require(seen.insert(value), "duplicate interned value")?;
        *states.entry(entry.state).or_default() += 1;
    }
    require(
        seen.contains("") && (0..=255).all(|point| seen.contains(&char::from(point).to_string())),
        "snapshot lacks empty or Latin-1 singleton entries",
    )?;
    let expected = serde_json::json!({"1": states.get(&1).copied().unwrap_or(0), "2": states.get(&2).copied().unwrap_or(0), "3": states.get(&3).copied().unwrap_or(0)});
    require(
        capture.pointer("/snapshot/states") == Some(&expected),
        "snapshot state counts differ from capture",
    )?;
    for (name, count) in [
        ("total", seen.len()),
        ("static_count", states.get(&3).copied().unwrap_or(0)),
        (
            "dynamic_count",
            states.get(&1).copied().unwrap_or(0) + states.get(&2).copied().unwrap_or(0),
        ),
    ] {
        require(
            capture
                .pointer(&format!("/snapshot/{name}"))
                .and_then(Value::as_u64)
                == Some(count as u64),
            "snapshot container count differs from capture",
        )?;
    }
    Ok(ValidatedSnapshot {
        entries: seen.len(),
        static_strings,
    })
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs_err::read(path).map_err(|error| error.to_string())
}

fn read_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|error| error.to_string())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
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
    use serde_json::json;

    fn contract() -> Value {
        let target = PythonVersion::Py314.target();
        let artifact = json!({"path": "/profile/artifact", "sha256": "unused"});
        json!({
            "schema_version": 1,
            "target": {"implementation": target.implementation, "version": target.version_string(), "tag": target.tag, "commit": target.commit, "magic_hex": target.magic_hex()},
            "compiler_input_mode": "source-bytes",
            "oracle": {"script_sha256": digest(b"oracle"), "arguments": ["-I", "-S", "-c"], "source_input": "bytes", "compile_mode": "exec", "dont_inherit": true, "optimize": 0, "capture_boundary": capture_boundary(PythonVersion::Py314)},
            "executable": artifact, "snapshot": artifact, "capture": artifact,
            "observer": artifact, "layout": artifact, "headers": artifact,
            "runtime_inputs": artifact, "process_maps": artifact, "runtime_capture": artifact
        })
    }

    #[test]
    fn binds_exact_target_oracle_boundary_and_compiler_mode() {
        let mut value = contract();
        let profile: Profile = serde_json::from_value(value.clone()).unwrap();
        assert!(
            profile
                .validate_contract(PythonVersion::Py314, true, "oracle")
                .is_ok()
        );
        assert!(
            profile
                .validate_contract(PythonVersion::Py314, false, "oracle")
                .is_err()
        );
        value["compiler_input_mode"] = json!("utf8-text");
        let profile: Profile = serde_json::from_value(value).unwrap();
        assert!(
            profile
                .validate_contract(PythonVersion::Py314, false, "oracle")
                .is_ok()
        );
        for (pointer, replacement) in [
            ("/schema_version", json!(2)),
            ("/target/commit", json!("different")),
            ("/oracle/script_sha256", json!(digest(b"different oracle"))),
            ("/oracle/arguments", json!(["-I", "-c"])),
            ("/oracle/source_input", json!("text")),
            ("/oracle/compile_mode", json!("eval")),
            ("/oracle/dont_inherit", json!(false)),
            ("/oracle/optimize", json!(1)),
            ("/oracle/capture_boundary", json!("before-bootstrap")),
        ] {
            let mut value = contract();
            *value.pointer_mut(pointer).unwrap() = replacement;
            let profile: Profile = serde_json::from_value(value).unwrap();
            assert!(
                profile
                    .validate_contract(PythonVersion::Py314, true, "oracle")
                    .is_err(),
                "{pointer}"
            );
        }
        let mut value = contract();
        value["unknown"] = json!(true);
        assert!(serde_json::from_value::<Profile>(value).is_err());
    }

    fn snapshot() -> (Vec<Value>, Value) {
        let mut entries = vec![json!({"pool":"static", "state":3, "codepoints":[]})];
        entries.extend(
            (0..=255).map(|point| json!({"pool":"static", "state":3, "codepoints":[point]})),
        );
        entries.push(json!({"pool":"dynamic", "state":1, "codepoints":[97,98]}));
        let capture = json!({"snapshot":{"states":{"1":1,"2":0,"3":257},"total":258,"static_count":257,"dynamic_count":1}});
        (entries, capture)
    }

    #[test]
    fn profiles_are_bound_to_the_selected_python_target() {
        for python_version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let target = python_version.target();
            let mut value = contract();
            value["target"] = json!({
                "implementation": target.implementation,
                "version": target.version_string(),
                "tag": target.tag,
                "commit": target.commit,
                "magic_hex": target.magic_hex(),
            });
            value["oracle"]["capture_boundary"] = json!(capture_boundary(python_version));
            let profile: Profile = serde_json::from_value(value.clone()).unwrap();
            assert!(
                profile
                    .validate_contract(python_version, true, "oracle")
                    .is_ok()
            );
            for other in [
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ] {
                assert_eq!(
                    profile.validate_contract(other, true, "oracle").is_ok(),
                    other == python_version
                );
            }
            assert!(
                profile
                    .validate_contract(python_version, true, "other oracle")
                    .is_err()
            );
            value["target"]["tag"] = json!("v3.15.0rc1");
            let profile: Profile = serde_json::from_value(value).unwrap();
            assert!(
                profile
                    .validate_contract(python_version, true, "oracle")
                    .is_err()
            );
        }
    }

    #[test]
    fn captured_dynamic_immortality_does_not_prove_cross_process_membership() {
        let (mut entries, mut capture) = snapshot();
        entries.push(json!({"pool":"dynamic", "state":2, "codepoints":[120,121]}));
        capture["snapshot"]["states"]["2"] = json!(1);
        capture["snapshot"]["total"] = json!(259);
        capture["snapshot"]["dynamic_count"] = json!(2);
        let snapshot =
            validate_snapshot(serde_json::from_value(json!(entries)).unwrap(), &capture).unwrap();
        assert_eq!(snapshot.entries, 259);
        assert_eq!(snapshot.static_strings.len(), 257);
        assert!(
            !snapshot
                .static_strings
                .iter()
                .any(|value| value == "xy" || value == "ab")
        );
    }

    #[test]
    fn validates_full_snapshot_and_rejects_incomplete_or_invalid_entries() {
        let (entries, capture) = snapshot();
        let validated =
            validate_snapshot(serde_json::from_value(json!(entries)).unwrap(), &capture).unwrap();
        assert_eq!(validated.entries, 258);
        assert_eq!(validated.static_strings.len(), 257);
        assert!(!validated.static_strings.iter().any(|value| value == "ab"));
        for change in 0..6 {
            let (mut entries, mut capture) = snapshot();
            match change {
                0 => {
                    entries.remove(0);
                }
                1 => entries.push(entries[0].clone()),
                2 => entries[0]["state"] = json!(1),
                3 => entries[257]["codepoints"] = json!([0xd800]),
                4 => capture["snapshot"]["total"] = json!(999),
                5 => capture["snapshot"]["states"]["1"] = json!(0),
                _ => unreachable!(),
            }
            assert!(
                validate_snapshot(serde_json::from_value(json!(entries)).unwrap(), &capture)
                    .is_err(),
                "change {change}"
            );
        }
    }

    #[test]
    fn rejects_content_drift_and_missing_runtime_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runtime");
        fs_err::write(&path, b"content").unwrap();
        let artifact = Artifact {
            path: path.clone(),
            sha256: digest(b"content"),
        };
        assert_eq!(artifact.read().unwrap(), b"content");
        let runtime = RuntimeInputs {
            runtime_prefix: directory.path().to_path_buf(),
            files: vec![RuntimeFile {
                path: path.clone(),
                sha256: digest(b"content"),
                size: 7,
                symlink_target: None,
            }],
        };
        assert!(runtime.validate().is_ok());
        assert!(
            runtime
                .validate_mapped_files(&format!("0-1 r-xp 0 00:00 0 {}\n", path.display()))
                .is_ok()
        );
        assert!(
            runtime
                .validate_mapped_files("0-1 r-xp 0 00:00 0 /missing/library\n")
                .is_err()
        );
        fs_err::write(directory.path().join("new-cache-file"), b"new").unwrap();
        assert!(runtime.validate().is_err());
        fs_err::remove_file(directory.path().join("new-cache-file")).unwrap();
        fs_err::write(&path, b"changed").unwrap();
        assert!(runtime.validate().is_err());
        assert!(artifact.read().is_err());
    }
}
