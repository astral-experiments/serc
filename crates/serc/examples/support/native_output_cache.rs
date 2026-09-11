//! Opt-in retention of the first comparison's bytes. This module never reads a cache.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{CompilerInput, CompilerOutcome, Outcome, OutcomeKind};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Artifact {
    pub(super) path: PathBuf,
    pub(super) sha256: String,
    pub(super) bytes: u64,
}

impl Artifact {
    pub(super) fn read(&self) -> io::Result<Vec<u8>> {
        let bytes = regular_bytes(&self.path)?;
        if bytes.len() as u64 != self.bytes || digest(&bytes) != self.sha256 {
            return Err(io::Error::other(format!(
                "changed cache input: {}",
                self.path.display()
            )));
        }
        Ok(bytes)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    output_directory: PathBuf,
    corpus_manifest: Artifact,
    compiler_source_manifest: Artifact,
    compiler_support_manifest: Artifact,
    compiler_build: Artifact,
}

pub(super) struct Writer {
    directory: PathBuf,
    cwd: PathBuf,
    native_identity_sha256: String,
    context: Artifact,
    inputs: Vec<Artifact>,
    paths: Vec<PathBuf>,
    records: Mutex<BTreeMap<usize, (Artifact, String)>>,
    errors: Mutex<Vec<String>>,
}

pub(super) struct Observation<'a> {
    writer: &'a Writer,
    index: usize,
    path: PathBuf,
    source: Option<Vec<u8>>,
    record: Value,
    errors: Vec<String>,
}

impl Writer {
    pub(super) fn new(
        config_path: &Path,
        profile_path: &Path,
        python: &str,
        oracle: &str,
        paths: &[PathBuf],
    ) -> io::Result<Self> {
        let config_bytes = regular_bytes(config_path)?;
        let config: Config = serde_json::from_slice(&config_bytes)?;
        let mut inputs = vec![artifact(config_path, &config_bytes)];
        for input in [
            &config.corpus_manifest,
            &config.compiler_source_manifest,
            &config.compiler_support_manifest,
            &config.compiler_build,
        ] {
            input.read()?;
            inputs.push(input.clone());
        }
        let profile_bytes = regular_bytes(profile_path)?;
        let profile: Value = serde_json::from_slice(&profile_bytes)?;
        let profile_artifact = artifact(profile_path, &profile_bytes);
        inputs.push(profile_artifact.clone());
        let executable = std::env::current_exe()?;
        let binary = regular_bytes(&executable)?;
        let compiler = artifact(&executable, &binary);
        inputs.push(compiler.clone());
        // Profile validation stays in the existing comparator. Its complete document
        // binds the target, runtime, executable, oracle and initial intern snapshot.
        let native_identity = json!({
            "profile_sha256": profile_artifact.sha256,
            "profile": profile,
            "python_argument": python,
            "oracle_script_sha256": digest(oracle.as_bytes()),
            "oracle_arguments": ["-I", "-S", "-c"],
            "source_input": "bytes", "compile_mode": "exec",
            "dont_inherit": true, "optimize": 0,
            "corpus_manifest_sha256": config.corpus_manifest.sha256,
        });
        let cwd = std::env::current_dir()?;
        let context = json!({
            "schema": 1, "native_identity": native_identity,
            "compiler": compiler, "provenance": config,
            "cwd": cwd, "cwd_os_bytes_hex": os_hex(cwd.as_os_str()),
            "argv_os_bytes_hex": std::env::args_os().map(|x| os_hex(&x)).collect::<Vec<_>>(),
            "selected_files": paths.len(),
            "requires_external_process_and_runtime_attestation": true,
            "cache_read_enabled": false,
        });
        Self::create(
            config.output_directory,
            &native_identity,
            &context,
            inputs,
            paths.to_vec(),
        )
    }

    fn create(
        directory: PathBuf,
        native_identity: &Value,
        context: &Value,
        inputs: Vec<Artifact>,
        paths: Vec<PathBuf>,
    ) -> io::Result<Self> {
        if !directory.is_absolute() {
            return Err(io::Error::other("cache output directory must be absolute"));
        }
        fs::create_dir(&directory)?;
        fs::create_dir(directory.join("blobs"))?;
        fs::create_dir(directory.join("entries"))?;
        let context_bytes = serde_json::to_vec(context)?;
        let context_path = directory.join("CONTEXT.json");
        write_new(&context_path, &context_bytes)?;
        Ok(Self {
            directory,
            cwd: std::env::current_dir()?,
            native_identity_sha256: digest(&serde_json::to_vec(native_identity)?),
            context: artifact(&context_path, &context_bytes),
            inputs,
            paths,
            records: Mutex::new(BTreeMap::new()),
            errors: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn begin(&self, index: usize, path: &Path) -> Observation<'_> {
        let mut observation = Observation {
            writer: self,
            index,
            path: path.to_path_buf(),
            source: None,
            record: json!({"index": index, "filename": path.to_string_lossy(),
                "source_argument_os_bytes_hex": os_hex(path.as_os_str()),
                "native": {"status": "not_attempted"},
                "compiler": {"status": "not_attempted"}}),
            errors: Vec::new(),
        };
        match fs::read(path) {
            Ok(bytes) => {
                observation.record["source"] =
                    json!({"sha256": digest(&bytes), "bytes": bytes.len()});
                observation.source = Some(bytes);
            }
            Err(error) => observation.errors.push(format!("source pre-read: {error}")),
        }
        observation
    }

    fn blob(&self, bytes: &[u8]) -> io::Result<Artifact> {
        let path = self.directory.join("blobs").join(digest(bytes));
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                if regular_bytes(&path)? != bytes {
                    return Err(io::Error::other("existing cache blob differs"));
                }
                return Ok(artifact(&path, bytes));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        match write_new(&path, bytes) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if regular_bytes(&path)? != bytes {
                    return Err(io::Error::other("existing cache blob differs"));
                }
            }
            Err(error) => return Err(error),
        }
        Ok(artifact(&path, bytes))
    }

    pub(super) fn finish(&self, comparison_requirements_satisfied: bool) -> io::Result<()> {
        if std::env::current_dir()? != self.cwd {
            return Err(io::Error::other("working directory changed during capture"));
        }
        let errors = self
            .errors
            .lock()
            .map_err(|_| io::Error::other("cache error lock poisoned"))?;
        if !errors.is_empty() {
            let _ = write_new(
                &self.directory.join("ERRORS.json"),
                &serde_json::to_vec(&*errors)?,
            );
            return Err(io::Error::other(errors.join("; ")));
        }
        let records = self
            .records
            .lock()
            .map_err(|_| io::Error::other("cache record lock poisoned"))?;
        if records.len() != self.paths.len() || !records.keys().copied().eq(0..self.paths.len()) {
            return Err(io::Error::other(
                "cache does not contain every selected input",
            ));
        }
        self.context.read()?;
        for input in &self.inputs {
            input.read()?;
        }
        let mut counts = BTreeMap::new();
        for (record, outcome) in records.values() {
            record.read()?;
            *counts.entry(outcome).or_insert(0_usize) += 1;
        }
        let complete = json!({
            "schema": 1, "status": "OUTPUT_CAPTURE_COMPLETE_REQUIRES_EXTERNAL_VALIDATION",
            "context": self.context, "files": self.paths.len(), "counts": counts,
            "records": records.values().map(|(record, _)| record).collect::<Vec<_>>(),
            "comparison_requirements_satisfied": comparison_requirements_satisfied,
            "actual_process_exit_attested": false, "cache_read_enabled": false,
        });
        write_new(
            &self.directory.join("COMPLETE.json"),
            &serde_json::to_vec(&complete)?,
        )
    }
}

impl Observation<'_> {
    pub(super) fn native(&mut self, output: &io::Result<Output>) {
        self.record["native"] = match output {
            Ok(output) => json!({
                "status": "observed", "exit_code": output.status.code(),
                "exit_status": output.status.to_string(), "success": output.status.success(),
                "stdout": self.keep(&output.stdout), "stderr": self.keep(&output.stderr),
            }),
            Err(error) => json!({"status": "spawn_error", "error": error.to_string()}),
        };
    }

    pub(super) fn compiled(&mut self, source: &CompilerInput, compiled: &CompilerOutcome) {
        if !matches!(source, CompilerInput::Bytes(bytes) if self.source.as_deref() == Some(bytes.as_slice()))
        {
            self.errors
                .push("compiler input differs from cache source pre-read".into());
        }
        self.record["compiler"] = match compiled {
            CompilerOutcome::Compiled(bytes) => {
                json!({"status": "compiled", "marshal": self.keep(bytes)})
            }
            CompilerOutcome::Unsupported(error) => json!({"status": "unsupported", "error": error}),
            CompilerOutcome::Rejected(error) => json!({"status": "rejected", "error": error}),
            CompilerOutcome::Failed(error) => json!({"status": "failed", "error": error}),
        };
    }

    fn keep(&mut self, bytes: &[u8]) -> Option<Artifact> {
        match self.writer.blob(bytes) {
            Ok(artifact) => Some(artifact),
            Err(error) => {
                self.errors.push(format!("cache blob: {error}"));
                None
            }
        }
    }

    pub(super) fn finish(mut self, outcome: &Outcome) {
        let result = (|| -> io::Result<()> {
            if self.writer.paths.get(self.index) != Some(&self.path) || outcome.path != self.path {
                return Err(io::Error::other("cache source/index/outcome mismatch"));
            }
            if std::env::current_dir()? != self.writer.cwd {
                return Err(io::Error::other(
                    "working directory changed during comparison",
                ));
            }
            if self.source.as_deref() != Some(fs::read(&self.path)?.as_slice()) {
                self.errors.push("source changed during comparison".into());
            }
            self.record["outcome"] = json!({"kind": outcome.kind.name(), "detail": outcome.detail,
                "oracle_rejected": outcome.oracle_rejected});
            self.record["literal_compiled_outputs_equal"] = json!(match outcome.kind {
                OutcomeKind::Exact => Some(true),
                OutcomeKind::ByteMismatch => Some(false),
                _ => None,
            });
            self.record["native_key_sha256"] = json!(native_key(
                &self.writer.native_identity_sha256,
                &self.writer.cwd,
                &self.path,
                &self.record["source"],
            )?);
            self.record["capture_errors"] = json!(self.errors);
            let path = self
                .writer
                .directory
                .join("entries")
                .join(format!("{:06}.json", self.index));
            let bytes = serde_json::to_vec(&self.record)?;
            write_new(&path, &bytes)?;
            self.writer
                .records
                .lock()
                .map_err(|_| io::Error::other("cache record lock poisoned"))?
                .insert(
                    self.index,
                    (artifact(&path, &bytes), outcome.kind.name().into()),
                );
            Ok(())
        })();
        if let Err(error) = result {
            self.errors.push(error.to_string());
        }
        if let Ok(mut errors) = self.writer.errors.lock() {
            errors.extend(
                self.errors
                    .into_iter()
                    .map(|error| format!("{}: {error}", self.path.display())),
            );
        }
    }
}

pub(super) fn native_key(
    identity_sha256: &str,
    cwd: &Path,
    filename: &Path,
    source: &Value,
) -> io::Result<String> {
    Ok(digest(&serde_json::to_vec(&json!({
        "native_identity_sha256": identity_sha256,
        "cwd_os_bytes_hex": os_hex(cwd.as_os_str()),
        "filename": filename.to_string_lossy(),
        "source_argument_os_bytes_hex": os_hex(filename.as_os_str()),
        "source": source,
    }))?))
}

pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn os_hex(value: &OsStr) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    value
        .as_encoded_bytes()
        .iter()
        .flat_map(|byte| {
            [
                char::from(HEX[(byte >> 4) as usize]),
                char::from(HEX[(byte & 15) as usize]),
            ]
        })
        .collect()
}

pub(super) fn artifact(path: &Path, bytes: &[u8]) -> Artifact {
    Artifact {
        path: path.to_path_buf(),
        sha256: digest(bytes),
        bytes: bytes.len() as u64,
    }
}

pub(super) fn regular_bytes(path: &Path) -> io::Result<Vec<u8>> {
    if !path.is_absolute() || !fs::symlink_metadata(path)?.is_file() {
        return Err(io::Error::other(
            "cache inputs and blobs must be absolute regular files",
        ));
    }
    fs::read(path)
}

pub(super) fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("cache path has no parent"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path).map_err(|error| error.error)?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::{OracleOutcome, OutcomeKind, classify, one_line};
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    fn writer(root: &Path, paths: &[PathBuf]) -> Writer {
        Writer::create(
            root.join("cache"),
            &json!({"fixture": true}),
            &json!({"fixture": true}),
            vec![],
            paths.to_vec(),
        )
        .unwrap()
    }

    fn observe(
        writer: &Writer,
        index: usize,
        status: i32,
        stdout: &[u8],
        stderr: &[u8],
        compiled: CompilerOutcome,
    ) -> Outcome {
        let path = &writer.paths[index];
        let mut record = writer.begin(index, path);
        record.native(&Ok(Output {
            status: ExitStatus::from_raw(status << 8),
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
        }));
        record.compiled(&CompilerInput::Bytes(fs::read(path).unwrap()), &compiled);
        let oracle = if status == 0 {
            OracleOutcome::Compiled(stdout.to_vec())
        } else {
            OracleOutcome::Rejected(one_line(stderr))
        };
        let outcome = classify(path, oracle, compiled);
        record.finish(&outcome);
        outcome
    }

    fn entry(writer: &Writer, index: usize) -> Value {
        serde_json::from_slice(
            &fs::read(
                writer
                    .directory
                    .join("entries")
                    .join(format!("{index:06}.json")),
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn concurrent_equal_outputs_share_bytes_but_keep_distinct_file_keys() {
        let root = tempfile::tempdir().unwrap();
        let paths = [root.path().join("first.py"), root.path().join("second.py")];
        for path in &paths {
            fs::write(path, "pass\n").unwrap();
        }
        let writer = writer(root.path(), &paths);
        std::thread::scope(|scope| {
            for index in 0..2 {
                let writer = &writer;
                scope.spawn(move || {
                    let outcome = observe(
                        writer,
                        index,
                        0,
                        b"actual marshal",
                        b"",
                        CompilerOutcome::Compiled(b"actual marshal".to_vec()),
                    );
                    assert_eq!(outcome.kind, OutcomeKind::Exact);
                });
            }
        });
        writer.finish(true).unwrap();
        let first = entry(&writer, 0);
        let second = entry(&writer, 1);
        assert_ne!(first["native_key_sha256"], second["native_key_sha256"]);
        assert_eq!(first["native"]["stdout"], first["compiler"]["marshal"]);
        assert_eq!(first["native"]["stdout"], second["native"]["stdout"]);
        assert_eq!(
            fs::read_dir(writer.directory.join("blobs"))
                .unwrap()
                .count(),
            2
        );
        let complete: Value =
            serde_json::from_slice(&fs::read(writer.directory.join("COMPLETE.json")).unwrap())
                .unwrap();
        assert_eq!(complete["files"], 2);
        assert_eq!(complete["counts"]["exact"], 2);
        assert_eq!(complete["actual_process_exit_attested"], false);
    }

    #[test]
    fn both_rejection_categories_retain_first_native_stderr_and_compiler_outcomes() {
        let root = tempfile::tempdir().unwrap();
        let paths = [
            root.path().join("rejected.py"),
            root.path().join("compiled.py"),
        ];
        for path in &paths {
            fs::write(path, "invalid fixture\n").unwrap();
        }
        let writer = writer(root.path(), &paths);
        let stderr = b"SyntaxError: original first line\nretained second line\n";
        assert_eq!(
            observe(
                &writer,
                0,
                2,
                b"",
                stderr,
                CompilerOutcome::Rejected("parser message".into())
            )
            .kind,
            OutcomeKind::MatchedRejection
        );
        assert_eq!(
            observe(
                &writer,
                1,
                2,
                b"",
                stderr,
                CompilerOutcome::Compiled(b"compiled anyway".to_vec())
            )
            .kind,
            OutcomeKind::RejectionMismatch
        );
        writer.finish(false).unwrap();
        let first = entry(&writer, 0);
        assert_eq!(first["native"]["exit_code"], 2);
        let artifact: Artifact = serde_json::from_value(first["native"]["stderr"].clone()).unwrap();
        assert_eq!(artifact.read().unwrap(), stderr);
        assert_eq!(entry(&writer, 1)["compiler"]["status"], "compiled");
    }

    #[test]
    fn source_drift_fails_capture_without_changing_the_comparison_outcome() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.py");
        fs::write(&path, "before\n").unwrap();
        let writer = writer(root.path(), std::slice::from_ref(&path));
        let mut record = writer.begin(0, &path);
        record.native(&Ok(Output {
            status: ExitStatus::from_raw(0),
            stdout: b"same".to_vec(),
            stderr: vec![],
        }));
        let compiled = CompilerOutcome::Compiled(b"same".to_vec());
        record.compiled(&CompilerInput::Bytes(b"before\n".to_vec()), &compiled);
        let outcome = classify(&path, OracleOutcome::Compiled(b"same".to_vec()), compiled);
        fs::write(&path, "after\n").unwrap();
        record.finish(&outcome);
        assert_eq!(outcome.kind, OutcomeKind::Exact);
        assert!(writer.finish(true).is_err());
        assert!(!writer.directory.join("COMPLETE.json").exists());
        assert!(writer.directory.join("ERRORS.json").exists());
    }

    #[test]
    fn blob_collision_does_not_overwrite_or_turn_capture_failure_into_success() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.py");
        fs::write(&path, "pass\n").unwrap();
        let writer = writer(root.path(), std::slice::from_ref(&path));
        let blob = writer.directory.join("blobs").join(digest(b"same"));
        fs::write(&blob, "wrong existing bytes").unwrap();
        assert_eq!(
            observe(
                &writer,
                0,
                0,
                b"same",
                b"",
                CompilerOutcome::Compiled(b"same".to_vec())
            )
            .kind,
            OutcomeKind::Exact
        );
        assert!(writer.finish(true).is_err());
        assert_eq!(fs::read(blob).unwrap(), b"wrong existing bytes");
        assert!(!writer.directory.join("COMPLETE.json").exists());
    }

    #[test]
    fn missing_and_duplicate_rows_never_publish_a_complete_capture() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.py");
        fs::write(&path, "pass\n").unwrap();
        let writer = writer(root.path(), std::slice::from_ref(&path));
        assert!(writer.finish(true).is_err());
        observe(
            &writer,
            0,
            0,
            b"same",
            b"",
            CompilerOutcome::Compiled(b"same".to_vec()),
        );
        let first = fs::read(writer.directory.join("entries/000000.json")).unwrap();
        observe(
            &writer,
            0,
            0,
            b"other",
            b"",
            CompilerOutcome::Compiled(b"other".to_vec()),
        );
        assert_eq!(
            fs::read(writer.directory.join("entries/000000.json")).unwrap(),
            first
        );
        assert!(writer.finish(true).is_err());
        assert!(!writer.directory.join("COMPLETE.json").exists());
        assert!(
            Writer::create(
                writer.directory.clone(),
                &json!({}),
                &json!({}),
                vec![],
                vec![]
            )
            .is_err()
        );
    }
}
