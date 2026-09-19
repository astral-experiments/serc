//! Explicitly admitted historical native output, always compared with fresh C.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::{Value, json};

use super::native_output_cache::{
    Artifact, artifact, digest, native_key, os_hex, regular_bytes, write_new,
};
use super::{
    CompilerInput, CompilerOutcome, InitialInternState, OracleOutcome, Outcome, OutcomeKind,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    expected_files: usize,
    expected_eligible: usize,
    expected_cpython_rejected: usize,
    admission: Artifact,
    admission_actual: Artifact,
    compiler_source_manifest: Artifact,
    compiler_support_manifest: Artifact,
    compiler_build: Artifact,
    output_directory: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    schema: u32,
    status: String,
    files: usize,
    eligible: usize,
    cpython_rejected: usize,
    native_identity: Value,
    corpus_manifest: Artifact,
    index: Artifact,
    closed_evidence: BTreeMap<String, Artifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexRow {
    corpus_index: usize,
    chunk: usize,
    entry: Artifact,
    native_key: String,
    filename: String,
    outcome: String,
    native_exit_code: i32,
}

struct Native {
    outcome: OracleOutcome,
    provenance: Value,
}

pub(super) struct Reader {
    cwd: PathBuf,
    directory: PathBuf,
    native_identity_sha256: String,
    index: BTreeMap<String, IndexRow>,
    inputs: Vec<Artifact>,
    context: Artifact,
    selected: Vec<PathBuf>,
    records: Mutex<BTreeMap<usize, (Artifact, String)>>,
    errors: Mutex<Vec<String>>,
}

fn require(condition: bool, message: &str) -> io::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message))
    }
}

fn value_artifact(value: &Value) -> io::Result<Artifact> {
    Ok(serde_json::from_value(value.clone())?)
}

impl Reader {
    pub(super) fn new(
        config_path: &Path,
        profile_path: &Path,
        python: &str,
        oracle: &str,
        paths: &[PathBuf],
    ) -> io::Result<Self> {
        let config_bytes = regular_bytes(config_path)?;
        let config: Config = serde_json::from_slice(&config_bytes)?;
        let admission_bytes = config.admission.read()?;
        let admission: Admission = serde_json::from_slice(&admission_bytes)?;
        let actual: Value = serde_json::from_slice(&config.admission_actual.read()?)?;
        require(
            actual["actual_exit_code"] == 0
                && actual["report"] == serde_json::to_value(&config.admission)?,
            "admission actual does not bind this admission",
        )?;
        require(
            admission.schema == 1
                && admission.status == "ADMITTED_NATIVE_OUTPUT_CACHE"
                && (
                    admission.files,
                    admission.eligible,
                    admission.cpython_rejected,
                ) == (
                    config.expected_files,
                    config.expected_eligible,
                    config.expected_cpython_rejected,
                )
                && admission.files > 0
                && admission.eligible > 0
                && admission.eligible.checked_add(admission.cpython_rejected) == Some(admission.files)
                && [
                    "capture",
                    "capture_actual",
                    "audit",
                    "audit_actual",
                    "window",
                    "window_actual",
                    "peer",
                    "peer_actual",
                ]
                .iter()
                .all(|role| admission.closed_evidence.contains_key(*role)),
            "native cache is not a complete admitted capture",
        )?;
        // Authenticity of these already closed tool receipts is checked by the
        // admission authority and the frozen caller, not inferred from booleans.
        let mut inputs = vec![
            artifact(config_path, &config_bytes),
            config.admission.clone(),
            config.admission_actual.clone(),
        ];
        for item in admission.closed_evidence.values().chain([
            &config.compiler_source_manifest,
            &config.compiler_support_manifest,
            &config.compiler_build,
            &admission.corpus_manifest,
            &admission.index,
        ]) {
            item.read()?;
            inputs.push(item.clone());
        }
        let profile_bytes = regular_bytes(profile_path)?;
        let profile: Value = serde_json::from_slice(&profile_bytes)?;
        inputs.push(artifact(profile_path, &profile_bytes));
        let native_identity = json!({
            "profile_sha256": digest(&profile_bytes), "profile": profile,
            "python_argument": python, "oracle_script_sha256": digest(oracle.as_bytes()),
            "oracle_arguments": ["-I", "-S", "-c"], "source_input": "bytes",
            "compile_mode": "exec", "dont_inherit": true, "optimize": 0,
            "corpus_manifest_sha256": admission.corpus_manifest.sha256,
        });
        require(
            native_identity == admission.native_identity,
            "native cache identity differs",
        )?;
        let manifest_bytes = admission.corpus_manifest.read()?;
        let manifest: Vec<Value> = manifest_bytes
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(serde_json::from_slice)
            .collect::<Result<_, _>>()?;
        require(
            manifest.len() == admission.files,
            "incomplete admitted corpus manifest",
        )?;
        let cwd = std::env::current_dir()?;
        let identity_sha256 = digest(&serde_json::to_vec(&native_identity)?);
        let index_bytes = admission.index.read()?;
        let mut index = BTreeMap::new();
        let mut keys = BTreeSet::new();
        let mut rejected = 0;
        for (position, line) in index_bytes
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .enumerate()
        {
            let row: IndexRow = serde_json::from_slice(line)?;
            let source = manifest
                .get(position)
                .ok_or_else(|| io::Error::other("extra cache index row"))?;
            let filename = format!(
                "target/corpus/files/{}",
                source["path"]
                    .as_str()
                    .ok_or_else(|| io::Error::other("invalid corpus filename"))?
            );
            require(
                row.corpus_index == position
                    && row.chunk == position / 4096
                    && row.filename == filename,
                "cache index membership or order differs",
            )?;
            require(
                matches!(row.native_exit_code, 0 | 2),
                "unusable native cache status",
            )?;
            let expected_key = native_key(
                &identity_sha256,
                &cwd,
                Path::new(&row.filename),
                &json!({"sha256": source["sha256"], "bytes": source["bytes"]}),
            )?;
            require(
                row.native_key == expected_key,
                "admitted index source/cwd key differs",
            )?;
            rejected += usize::from(row.native_exit_code == 2);
            require(
                keys.insert(row.native_key.clone()),
                "duplicate native cache key",
            )?;
            require(
                index.insert(row.filename.clone(), row).is_none(),
                "duplicate cache filename",
            )?;
        }
        require(
            index.len() == admission.files && rejected == admission.cpython_rejected,
            "incomplete cache index",
        )?;
        for path in paths {
            require(
                index.contains_key(&path.to_string_lossy().into_owned()),
                "selected file is not in admitted cache",
            )?;
        }
        require(
            config.output_directory.is_absolute(),
            "reader output must be absolute",
        )?;
        let executable = std::env::current_exe()?;
        let compiler_bytes = regular_bytes(&executable)?;
        let compiler = artifact(&executable, &compiler_bytes);
        inputs.push(compiler.clone());
        require(
            std::env::current_dir()? == cwd,
            "reader cwd changed while loading",
        )?;
        let context = json!({
            "schema": 1, "cache_read_enabled": true, "admission": config.admission,
            "admission_actual": config.admission_actual, "native_identity": native_identity,
            "compiler": compiler, "compiler_source_manifest": config.compiler_source_manifest,
            "compiler_support_manifest": config.compiler_support_manifest, "compiler_build": config.compiler_build,
            "cwd": cwd, "cwd_os_bytes_hex": os_hex(cwd.as_os_str()),
            "argv_os_bytes_hex": std::env::args_os().map(|a| os_hex(&a)).collect::<Vec<_>>(),
            "selected_files": paths.len(), "fresh_compiler_output_required": true,
            "native_provenance": "historical admitted capture; no per-file native process executed",
            "external_admission_authenticity_and_current_runtime_guards_required": true,
        });
        fs::create_dir(&config.output_directory)?;
        fs::create_dir(config.output_directory.join("blobs"))?;
        fs::create_dir(config.output_directory.join("entries"))?;
        let context_bytes = serde_json::to_vec(&context)?;
        let context_path = config.output_directory.join("CONTEXT.json");
        write_new(&context_path, &context_bytes)?;
        Ok(Self {
            cwd,
            directory: config.output_directory,
            native_identity_sha256: digest(&serde_json::to_vec(&native_identity)?),
            index,
            inputs,
            context: artifact(&context_path, &context_bytes),
            selected: paths.to_vec(),
            records: Mutex::new(BTreeMap::new()),
            errors: Mutex::new(Vec::new()),
        })
    }

    fn lookup(&self, path: &Path, source: &[u8]) -> io::Result<Native> {
        require(std::env::current_dir()? == self.cwd, "reader cwd changed")?;
        let filename = path.to_string_lossy();
        let row = self
            .index
            .get(filename.as_ref())
            .ok_or_else(|| io::Error::other("native cache key missing"))?;
        let source_identity = json!({"sha256": digest(source), "bytes": source.len()});
        require(
            native_key(
                &self.native_identity_sha256,
                &self.cwd,
                path,
                &source_identity,
            )? == row.native_key,
            "native cache source key differs",
        )?;
        let entry: Value = serde_json::from_slice(&row.entry.read()?)?;
        require(
            entry["filename"] == row.filename
                && entry["source_argument_os_bytes_hex"] == os_hex(path.as_os_str())
                && entry["source"] == source_identity
                && entry["native_key_sha256"] == row.native_key
                && entry["capture_errors"] == json!([]),
            "cached entry identity differs",
        )?;
        let native = &entry["native"];
        require(
            native["status"] == "observed"
                && native["exit_code"] == row.native_exit_code
                && native["success"] == (row.native_exit_code == 0)
                && native["exit_status"] == format!("exit status: {}", row.native_exit_code),
            "cached native exit differs",
        )?;
        let stdout = value_artifact(&native["stdout"])?;
        let stderr = value_artifact(&native["stderr"])?;
        let entry_directory = row
            .entry
            .path
            .parent()
            .ok_or_else(|| io::Error::other("invalid entry path"))?;
        require(
            entry_directory
                .file_name()
                .is_some_and(|name| name == "entries"),
            "invalid entry directory",
        )?;
        let blobs = entry_directory
            .parent()
            .ok_or_else(|| io::Error::other("invalid cache directory"))?
            .join("blobs");
        for item in [&stdout, &stderr] {
            require(
                item.path.parent() == Some(blobs.as_path())
                    && item
                        .path
                        .file_name()
                        .is_some_and(|name| name == item.sha256.as_str()),
                "cached blob is outside its content-addressed directory",
            )?;
        }
        let output = stdout.read()?;
        let error = stderr.read()?;
        let outcome = if row.native_exit_code == 0 {
            OracleOutcome::Compiled(output)
        } else {
            require(
                output.is_empty(),
                "native rejection contains unexpected marshal output",
            )?;
            OracleOutcome::Rejected(super::one_line(&error))
        };
        Ok(Native {
            outcome,
            provenance: json!({
                "entry": row.entry, "native_key": row.native_key,
                "stdout": stdout, "stderr": stderr, "historical_exit_code": row.native_exit_code,
                "historical_outcome": row.outcome, "fresh_native_process_executed": false,
            }),
        })
    }

    fn blob(&self, bytes: &[u8]) -> io::Result<Artifact> {
        let path = self.directory.join("blobs").join(digest(bytes));
        match write_new(&path, bytes) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => require(
                regular_bytes(&path)? == bytes,
                "reader output blob conflict",
            )?,
            Err(error) => return Err(error),
        }
        Ok(artifact(&path, bytes))
    }

    pub(super) fn compare(
        &self,
        index: usize,
        path: &Path,
        state: Option<&InitialInternState>,
    ) -> Outcome {
        self.compare_with(index, path, |input, filename| {
            super::compile_input(input, filename, state)
        })
    }

    fn compare_with(
        &self,
        index: usize,
        path: &Path,
        compile: impl FnOnce(&CompilerInput, &str) -> CompilerOutcome,
    ) -> Outcome {
        let result = (|| -> io::Result<Outcome> {
            require(
                self.selected.get(index).is_some_and(|p| p == path),
                "reader selection changed",
            )?;
            let bytes = fs::read(path)?;
            let native = self.lookup(path, &bytes)?;
            let source_identity = json!({"sha256": digest(&bytes), "bytes": bytes.len()});
            let input = CompilerInput::Bytes(bytes);
            // Always invoke the current compiler. The cached compiler field is never read.
            let compiled = compile(&input, &path.to_string_lossy());
            let compiler = match &compiled {
                CompilerOutcome::Compiled(bytes) => {
                    json!({"status": "compiled", "marshal": self.blob(bytes)?})
                }
                CompilerOutcome::Rejected(error) => json!({"status": "rejected", "error": error}),
                CompilerOutcome::Unsupported(error) => {
                    json!({"status": "unsupported", "error": error})
                }
                CompilerOutcome::Failed(error) => json!({"status": "failed", "error": error}),
            };
            let after = fs::read(path)?;
            require(
                source_identity == json!({"sha256": digest(&after), "bytes": after.len()}),
                "reader source changed during compilation",
            )?;
            let outcome = super::classify(path, native.outcome, compiled);
            let entry = json!({"index": index, "filename": path.to_string_lossy(),
                "source_argument_os_bytes_hex": os_hex(path.as_os_str()), "source": source_identity,
                "cached_native": native.provenance, "fresh_compiler": compiler,
                "outcome": {"kind": outcome.kind.name(), "detail": outcome.detail, "oracle_rejected": outcome.oracle_rejected}});
            let raw = serde_json::to_vec(&entry)?;
            let destination = self
                .directory
                .join("entries")
                .join(format!("{index:06}.json"));
            write_new(&destination, &raw)?;
            let mut records = self
                .records
                .lock()
                .map_err(|_| io::Error::other("reader record lock poisoned"))?;
            require(
                records
                    .insert(
                        index,
                        (
                            artifact(&destination, &raw),
                            outcome.kind.name().to_string(),
                        ),
                    )
                    .is_none(),
                "duplicate reader output",
            )?;
            Ok(outcome)
        })();
        match result {
            Ok(outcome) => outcome,
            Err(error) => {
                if let Ok(mut errors) = self.errors.lock() {
                    errors.push(format!("{}: {error}", path.display()));
                }
                Outcome {
                    path: path.to_path_buf(),
                    kind: OutcomeKind::OracleFailure,
                    detail: format!("admitted native cache failed: {error}"),
                    oracle_rejected: false,
                }
            }
        }
    }

    pub(super) fn finish(&self, gates_passed: bool) -> io::Result<()> {
        let errors = self
            .errors
            .lock()
            .map_err(|_| io::Error::other("reader error lock poisoned"))?;
        require(errors.is_empty(), &errors.join("; "))?;
        require(std::env::current_dir()? == self.cwd, "reader cwd changed")?;
        for input in &self.inputs {
            input.read()?;
        }
        let records = self
            .records
            .lock()
            .map_err(|_| io::Error::other("reader record lock poisoned"))?;
        require(
            records.len() == self.selected.len()
                && records.keys().copied().eq(0..self.selected.len()),
            "reader output is incomplete",
        )?;
        let mut counts = BTreeMap::<&str, usize>::new();
        for (_, kind) in records.values() {
            *counts.entry(kind).or_default() += 1;
        }
        let complete = json!({"schema": 1, "status": "FRESH_COMPILER_VS_ADMITTED_NATIVE_COMPLETE_REQUIRES_EXTERNAL_VALIDATION",
            "context": self.context, "files": self.selected.len(), "counts": counts,
            "records": records.values().map(|(a, _)| a).collect::<Vec<_>>(),
            "comparison_requirements_satisfied": gates_passed, "actual_process_exit_attested": false,
            "cache_read_enabled": true, "cached_compiler_used_as_candidate": false,
            "per_file_native_processes_executed": 0});
        write_new(
            &self.directory.join("COMPLETE.json"),
            &serde_json::to_vec(&complete)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Internal synthetic vectors only. This does not mint or load an admission.
    fn fixture(rejected: bool) -> (tempfile::TempDir, Reader, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source_é.py");
        fs::write(&source, b"x = 1\n").unwrap();
        let cache = dir.path().join("historical");
        fs::create_dir(&cache).unwrap();
        fs::create_dir(cache.join("entries")).unwrap();
        fs::create_dir(cache.join("blobs")).unwrap();
        let historical_blob = |bytes: &[u8]| {
            let p = cache.join("blobs").join(digest(bytes));
            if !p.exists() {
                write_new(&p, bytes).unwrap();
            }
            artifact(&p, bytes)
        };
        let stdout = historical_blob(if rejected { b"" } else { b"native" });
        let stderr = historical_blob(if rejected {
            b"SyntaxError: synthetic rejection\n"
        } else {
            b""
        });
        let cwd = std::env::current_dir().unwrap();
        let identity = digest(b"synthetic unit identity");
        let source_identity = json!({"sha256": digest(b"x = 1\n"), "bytes": 6});
        let key = digest(&serde_json::to_vec(&json!({
            "native_identity_sha256": identity, "cwd_os_bytes_hex": os_hex(cwd.as_os_str()),
            "filename": source.to_string_lossy(), "source_argument_os_bytes_hex": os_hex(source.as_os_str()),
            "source": source_identity,
        })).unwrap());
        let code = if rejected { 2 } else { 0 };
        let entry = json!({
            "filename": source.to_string_lossy(), "source_argument_os_bytes_hex": os_hex(source.as_os_str()),
            "source": source_identity, "native_key_sha256": key, "capture_errors": [],
            "native": {"status": "observed", "exit_code": code, "exit_status": format!("exit status: {code}"),
                "success": !rejected, "stdout": stdout, "stderr": stderr},
            "compiler": {"status": "compiled", "marshal": {"path": "/deliberately-missing-cached-compiler", "sha256": "unused", "bytes": 1}},
        });
        let entry_bytes = serde_json::to_vec(&entry).unwrap();
        let entry_path = cache.join("entries/000000.json");
        write_new(&entry_path, &entry_bytes).unwrap();
        let output = dir.path().join("current");
        fs::create_dir(&output).unwrap();
        fs::create_dir(output.join("blobs")).unwrap();
        fs::create_dir(output.join("entries")).unwrap();
        let context_path = output.join("CONTEXT.json");
        write_new(&context_path, b"synthetic unit context").unwrap();
        let filename = source.to_string_lossy().into_owned();
        let row = IndexRow {
            corpus_index: 0,
            chunk: 0,
            entry: artifact(&entry_path, &entry_bytes),
            native_key: key,
            filename: filename.clone(),
            outcome: "synthetic".into(),
            native_exit_code: code,
        };
        let reader = Reader {
            cwd,
            directory: output,
            native_identity_sha256: identity,
            index: BTreeMap::from([(filename, row)]),
            inputs: Vec::new(),
            context: artifact(&context_path, b"synthetic unit context"),
            selected: vec![source.clone()],
            records: Mutex::new(BTreeMap::new()),
            errors: Mutex::new(Vec::new()),
        };
        (dir, reader, source)
    }

    #[test]
    fn always_uses_fresh_candidate_and_never_reads_cached_compiler() {
        let (_dir, reader, source) = fixture(false);
        let calls = AtomicUsize::new(0);
        let outcome = reader.compare_with(0, &source, |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            CompilerOutcome::Compiled(b"native".to_vec())
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(outcome.kind, OutcomeKind::Exact);
        reader.finish(true).unwrap();
        let entry: Value = serde_json::from_slice(
            &fs::read(reader.directory.join("entries/000000.json")).unwrap(),
        )
        .unwrap();
        let current = value_artifact(&entry["fresh_compiler"]["marshal"]).unwrap();
        assert_eq!(current.read().unwrap(), b"native");
        assert_eq!(
            entry["cached_native"]["fresh_native_process_executed"],
            false
        );
    }

    #[test]
    fn retained_rejection_is_not_a_fabricated_new_process() {
        let (_dir, reader, source) = fixture(true);
        let outcome = reader.compare_with(0, &source, |_, _| {
            CompilerOutcome::Compiled(b"fresh".to_vec())
        });
        assert_eq!(outcome.kind, OutcomeKind::RejectionMismatch);
        reader.finish(true).unwrap();
        let entry: Value = serde_json::from_slice(
            &fs::read(reader.directory.join("entries/000000.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(entry["cached_native"]["historical_exit_code"], 2);
        assert_eq!(
            entry["cached_native"]["fresh_native_process_executed"],
            false
        );
        assert!(entry["cached_native"].get("pid").is_none());
    }

    #[test]
    fn changed_source_or_missing_key_fails_without_fallback() {
        for missing in [false, true] {
            let (_dir, mut reader, source) = fixture(false);
            if missing {
                reader.index.clear();
            } else {
                fs::write(&source, b"x = 2\n").unwrap();
            }
            let outcome = reader.compare_with(0, &source, |_, _| {
                panic!("invalid cache must not compile or fall back")
            });
            assert_eq!(outcome.kind, OutcomeKind::OracleFailure);
            assert!(reader.finish(true).is_err());
            assert!(!reader.directory.join("COMPLETE.json").exists());
        }
    }

    #[test]
    fn corrupt_native_blob_fails_closed() {
        let (_dir, reader, source) = fixture(false);
        let row = reader.index.values().next().unwrap();
        let entry: Value = serde_json::from_slice(&row.entry.read().unwrap()).unwrap();
        let stdout = value_artifact(&entry["native"]["stdout"]).unwrap();
        fs::write(stdout.path, b"tampered").unwrap();
        let outcome = reader.compare_with(0, &source, |_, _| {
            panic!("corrupt cache must not fall back")
        });
        assert_eq!(outcome.kind, OutcomeKind::OracleFailure);
        assert!(reader.finish(true).is_err());
    }

    #[test]
    fn source_drift_after_fresh_compilation_prevents_completion() {
        let (_dir, reader, source) = fixture(false);
        let outcome = reader.compare_with(0, &source, |_, _| {
            fs::write(&source, b"changed after compilation").unwrap();
            CompilerOutcome::Compiled(b"native".to_vec())
        });
        assert_eq!(outcome.kind, OutcomeKind::OracleFailure);
        assert!(reader.finish(true).is_err());
    }
}
