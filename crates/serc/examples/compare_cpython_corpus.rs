#![allow(clippy::print_stderr, clippy::print_stdout)]

use std::any::Any;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

use clap::Parser;
use rayon::prelude::*;
use serc::{
    CompileError, CompileOptions, OptimizationLevel, PythonVersion, compile_bytes_with_options,
};
use walkdir::WalkDir;

const WORKER_STACK_SIZE: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum OutcomeKind {
    Exact,
    ByteMismatch,
    Unsupported,
    ParseMismatch,
    CompilerFailure,
    MatchedRejection,
    RejectionMismatch,
    OracleFailure,
}

impl OutcomeKind {
    const ALL: [Self; 8] = [
        Self::Exact,
        Self::ByteMismatch,
        Self::Unsupported,
        Self::ParseMismatch,
        Self::CompilerFailure,
        Self::MatchedRejection,
        Self::RejectionMismatch,
        Self::OracleFailure,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::ByteMismatch => "byte mismatch",
            Self::Unsupported => "unsupported",
            Self::ParseMismatch => "parse/decode mismatch",
            Self::CompilerFailure => "compiler failure",
            Self::MatchedRejection => "matched rejection",
            Self::RejectionMismatch => "rejection mismatch",
            Self::OracleFailure => "oracle failure",
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::ByteMismatch => "byte-mismatch",
            Self::Unsupported => "unsupported",
            Self::ParseMismatch => "parse-mismatch",
            Self::CompilerFailure => "compiler-failure",
            Self::MatchedRejection => "matched-rejection",
            Self::RejectionMismatch => "rejection-mismatch",
            Self::OracleFailure => "oracle-failure",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    const fn is_failure(self) -> bool {
        !matches!(self, Self::Exact | Self::MatchedRejection)
    }
}

#[derive(Debug)]
struct Outcome {
    path: PathBuf,
    kind: OutcomeKind,
    detail: String,
    oracle_rejected: bool,
    mismatch_bytes: Option<MismatchBytes>,
}

#[derive(Debug)]
struct MismatchBytes {
    ours: Vec<u8>,
    oracle: Vec<u8>,
}

enum OracleOutcome {
    Compiled(Vec<u8>),
    Rejected(String),
}

enum CompilerOutcome {
    Compiled(Vec<u8>),
    Unsupported(String),
    Rejected(String),
    Failed(String),
}

#[derive(Debug, Parser)]
#[command(about = "Compare a Python corpus against the pinned CPython target")]
struct Options {
    /// Use a fresh minimal CPython reference process for each source file.
    #[arg(long, value_name = "PATH")]
    reference: PathBuf,
    /// Compile at this optimization level in the minimal reference process and Serc.
    #[arg(long, default_value_t = OptimizationLevel::default(), value_name = "0|1|2")]
    optimize: OptimizationLevel,
    #[arg(long, default_value_t = PythonVersion::default(), value_name = "3.12|3.13|3.14|3.15")]
    python_version: PythonVersion,
    #[arg(value_name = "ROOT", default_values_os_t = default_roots())]
    roots: Vec<PathBuf>,
    /// Additional file extensions to include alongside .py.
    #[arg(long = "extension", value_parser = parse_extension, value_name = "EXT")]
    extensions: Vec<String>,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long)]
    expected_files: Option<usize>,
    #[arg(long = "expected-outcome", value_parser = parse_expected_outcome, value_name = "KIND=COUNT")]
    expected_outcomes: Vec<(OutcomeKind, usize)>,
    #[arg(long)]
    expected_exact: Option<usize>,
    #[arg(long)]
    expected_cpython_rejected: Option<usize>,
    /// Require every CPython-accepted input to match exactly.
    #[arg(long)]
    require_all: bool,
    #[arg(long)]
    require_rejection_parity: bool,
    #[arg(long, default_value_t = 10)]
    examples: usize,
    #[arg(long, value_name = "DIR")]
    dump_mismatches: Option<PathBuf>,
}

impl Options {
    fn expected_outcome_counts(&self) -> Result<BTreeMap<OutcomeKind, usize>, String> {
        let mut outcomes = BTreeMap::new();
        for (kind, count) in self
            .expected_outcomes
            .iter()
            .copied()
            .chain(self.expected_exact.map(|count| (OutcomeKind::Exact, count)))
        {
            if outcomes.insert(kind, count).is_some() {
                return Err(format!(
                    "expected outcome `{}` may only be specified once",
                    kind.name()
                ));
            }
        }
        Ok(outcomes)
    }
}

fn main() -> ExitCode {
    let options = Options::parse();
    let expected_outcomes = match options.expected_outcome_counts() {
        Ok(outcomes) => outcomes,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    if let Err(message) = verify_reference(&options.reference, options.python_version) {
        eprintln!("{message}");
        return ExitCode::from(2);
    }
    let compile_options = CompileOptions {
        python_version: options.python_version,
        optimization_level: options.optimize,
        ..CompileOptions::default()
    };
    let mut paths = match discover(&options.roots, &options.extensions) {
        Ok(paths) => paths,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    if paths.is_empty() {
        eprintln!("the corpus roots contain no Python files");
        return ExitCode::from(2);
    }
    if let Some(expected) = options.expected_files
        && paths.len() != expected
    {
        eprintln!(
            "discovered {} Python files, but the frozen corpus requires {expected}",
            paths.len()
        );
        return ExitCode::from(2);
    }
    if let Some(limit) = options.limit {
        paths.truncate(limit);
    }
    let pool = match rayon::ThreadPoolBuilder::new()
        .stack_size(WORKER_STACK_SIZE)
        .build()
    {
        Ok(pool) => pool,
        Err(error) => {
            eprintln!("failed to create the comparison worker pool: {error}");
            return ExitCode::from(2);
        }
    };

    // Stage mismatches in the destination filesystem so that the original
    // bytes do not all have to remain in memory until reporting finishes.
    let dump_stage = match options.dump_mismatches.as_deref() {
        Some(directory) => {
            match fs_err::create_dir_all(directory).and_then(|()| tempfile::tempdir_in(directory)) {
                Ok(stage) => Some(stage),
                Err(error) => {
                    eprintln!("failed to prepare mismatch dumps: {error}");
                    return ExitCode::FAILURE;
                }
            }
        }
        None => None,
    };

    let started = Instant::now();
    let outcomes: Result<Vec<_>, std::io::Error> = pool.install(|| {
        paths
            .par_iter()
            .enumerate()
            .map(|(index, path)| {
                let mut outcome = compare(
                    path,
                    &options.reference,
                    &compile_options,
                    options.dump_mismatches.is_some(),
                );
                if let Some(stage) = &dump_stage {
                    stage_mismatch(&mut outcome, stage.path(), index)?;
                }
                Ok(outcome)
            })
            .collect()
    });
    let outcomes = match outcomes {
        Ok(outcomes) => outcomes,
        Err(error) => {
            eprintln!("failed to stage mismatch dumps: {error}");
            return ExitCode::FAILURE;
        }
    };

    let mut counts = BTreeMap::<OutcomeKind, usize>::new();
    for outcome in &outcomes {
        *counts.entry(outcome.kind).or_default() += 1;
    }
    let cpython_rejected = outcomes
        .iter()
        .filter(|outcome| outcome.oracle_rejected)
        .count();

    println!(
        "Compared {} Python files with {} in {:.2?}",
        outcomes.len(),
        options.reference.display(),
        started.elapsed()
    );
    println!(
        "{:>20}: {}",
        OutcomeKind::Exact.label(),
        counts.get(&OutcomeKind::Exact).copied().unwrap_or_default()
    );
    println!("{:>20}: {cpython_rejected}", "CPython rejected");
    for kind in [
        OutcomeKind::MatchedRejection,
        OutcomeKind::RejectionMismatch,
        OutcomeKind::ByteMismatch,
        OutcomeKind::Unsupported,
        OutcomeKind::ParseMismatch,
        OutcomeKind::CompilerFailure,
        OutcomeKind::OracleFailure,
    ] {
        let count = counts.get(&kind).copied().unwrap_or_default();
        println!("{:>20}: {count}", kind.label());
    }

    for kind in [
        OutcomeKind::ByteMismatch,
        OutcomeKind::Unsupported,
        OutcomeKind::ParseMismatch,
        OutcomeKind::CompilerFailure,
        OutcomeKind::RejectionMismatch,
        OutcomeKind::OracleFailure,
    ] {
        let examples: Vec<_> = outcomes
            .iter()
            .filter(|outcome| outcome.kind == kind)
            .take(options.examples)
            .collect();
        if examples.is_empty() {
            continue;
        }
        println!("\n{} examples:", kind.label());
        for outcome in examples {
            println!("  {}: {}", outcome.path.display(), outcome.detail);
        }
    }

    if let Some(directory) = &options.dump_mismatches {
        let stage = dump_stage.as_ref().expect("dump stage initialized");
        if let Err(error) = pool.install(|| dump_mismatches(&outcomes, stage.path(), directory)) {
            eprintln!("failed to dump mismatches: {error}");
            return ExitCode::FAILURE;
        }
    }

    let unsupported_reasons = outcomes
        .iter()
        .filter(|outcome| outcome.kind == OutcomeKind::Unsupported)
        .fold(BTreeMap::<&str, usize>::new(), |mut reasons, outcome| {
            *reasons.entry(&outcome.detail).or_default() += 1;
            reasons
        });
    if !unsupported_reasons.is_empty() {
        let mut unsupported_reasons: Vec<_> = unsupported_reasons.into_iter().collect();
        unsupported_reasons.sort_by_key(|(message, count)| (std::cmp::Reverse(*count), *message));
        println!("\nunsupported reasons:");
        for (message, count) in unsupported_reasons {
            println!("  {count:>5}  {message}");
        }
    }

    let gate_failures = validate_outcome_counts(
        &counts,
        cpython_rejected,
        outcomes
            .iter()
            .filter(|outcome| !outcome.oracle_rejected && outcome.kind.is_failure())
            .count(),
        outcomes
            .iter()
            .filter(|outcome| outcome.oracle_rejected && outcome.kind.is_failure())
            .count(),
        &expected_outcomes,
        options.expected_cpython_rejected,
        options.require_all,
        options.require_rejection_parity,
    );
    for failure in &gate_failures {
        eprintln!("compatibility gate failed: {failure}");
    }
    if gate_failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn validate_outcome_counts(
    counts: &BTreeMap<OutcomeKind, usize>,
    cpython_rejected: usize,
    cpython_accepted_failures: usize,
    rejection_failures: usize,
    expected_outcomes: &BTreeMap<OutcomeKind, usize>,
    expected_cpython_rejected: Option<usize>,
    require_all: bool,
    require_rejection_parity: bool,
) -> Vec<String> {
    let count = |kind| counts.get(&kind).copied().unwrap_or_default();
    let exact = count(OutcomeKind::Exact);
    let mut failures = Vec::new();

    for (&kind, &expected) in expected_outcomes {
        let actual = count(kind);
        if actual == expected {
            continue;
        }
        failures.push(match kind {
            OutcomeKind::Exact => {
                format!("found {actual} exact matches, but expected {expected}")
            }
            _ => format!(
                "found {actual} {} outcomes, but expected {expected}",
                kind.label()
            ),
        });
    }
    if let Some(expected) = expected_cpython_rejected
        && cpython_rejected != expected
    {
        failures.push(format!(
            "CPython rejected {cpython_rejected} files, but expected {expected}"
        ));
    }
    if require_all {
        if cpython_accepted_failures != 0 {
            failures.push(format!(
                "{cpython_accepted_failures} CPython-accepted files did not match exactly"
            ));
        }
        if exact == 0 {
            failures.push("no files matched CPython exactly".to_string());
        }
    }
    if require_rejection_parity && rejection_failures != 0 {
        failures.push(format!(
            "{rejection_failures} CPython-rejected files were not rejected cleanly by Serc"
        ));
    }

    failures
}

#[derive(Debug, serde::Deserialize)]
struct ReferenceIdentity {
    protocol: String,
    implementation: String,
    version: String,
    release_level: String,
    release_serial: u8,
    magic: String,
}

impl ReferenceIdentity {
    fn matches(&self, version: PythonVersion) -> bool {
        let target = version.target();
        let (release_level, release_serial) = match version {
            PythonVersion::Py312 | PythonVersion::Py313 | PythonVersion::Py314 => ("final", 0),
            PythonVersion::Py315 => ("candidate", 2),
        };
        self.protocol == "minimal-v2"
            && self.implementation == target.implementation
            && self.version == target.version_string()
            && self.release_level == release_level
            && self.release_serial == release_serial
            && self.magic == target.magic_hex()
    }
}

/// Checks the protocol and linked runtime of the executable used for compilation.
fn verify_reference(reference: &Path, python_version: PythonVersion) -> Result<(), String> {
    let output = Command::new(reference)
        .arg("--identity")
        .output()
        .map_err(|error| format!("failed to run {}: {error}", reference.display()))?;
    let identity: ReferenceIdentity = serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "failed to read {} identity: {error}; {}",
            reference.display(),
            one_line(&output.stderr)
        )
    })?;
    if output.status.success() && identity.matches(python_version) {
        Ok(())
    } else {
        Err(format!(
            "{} is {identity:?}; the reference requires minimal-v2 and {} with magic {}",
            reference.display(),
            python_version.target().tag,
            python_version.target().magic_hex()
        ))
    }
}

fn default_roots() -> Vec<PathBuf> {
    let ruff = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/ruff");
    [
        "crates/ty_python_semantic/resources/corpus",
        "crates/ruff_python_parser/resources/valid",
        "crates/ruff_python_parser/resources/inline/ok",
        "crates/ruff_linter/resources/test/fixtures",
        "crates/ruff_python_formatter/resources/test/fixtures",
    ]
    .into_iter()
    .map(|root| ruff.join(root))
    .collect()
}

fn parse_extension(extension: &str) -> Result<String, String> {
    if extension.is_empty()
        || extension.starts_with('.')
        || extension.contains(std::path::is_separator)
    {
        return Err("expected a non-empty bare extension such as `pyi`".to_string());
    }
    Ok(extension.to_string())
}

fn parse_expected_outcome(value: &str) -> Result<(OutcomeKind, usize), String> {
    let (name, count) = value
        .split_once('=')
        .ok_or_else(|| "--expected-outcome requires a KIND=COUNT value".to_string())?;
    let kind = OutcomeKind::from_name(name).ok_or_else(|| {
        let names = OutcomeKind::ALL.map(OutcomeKind::name).join(", ");
        format!("unknown outcome kind `{name}`; expected one of: {names}")
    })?;
    let count = count
        .parse()
        .map_err(|_| "--expected-outcome count must be a number".to_string())?;
    Ok((kind, count))
}

fn stage_mismatch(outcome: &mut Outcome, stage: &Path, index: usize) -> Result<(), std::io::Error> {
    if outcome.kind != OutcomeKind::ByteMismatch {
        return Ok(());
    }
    let bytes = outcome
        .mismatch_bytes
        .take()
        .ok_or_else(|| std::io::Error::other("original mismatch bytes were not retained"))?;
    fs_err::write(stage.join(format!("{index}.ours")), bytes.ours)?;
    fs_err::write(stage.join(format!("{index}.cpython")), bytes.oracle)
}

fn dump_mismatches(
    outcomes: &[Outcome],
    stage: &Path,
    directory: &Path,
) -> Result<(), std::io::Error> {
    outcomes
        .iter()
        .enumerate()
        .filter(|(_, outcome)| outcome.kind == OutcomeKind::ByteMismatch)
        .collect::<Vec<_>>()
        .into_par_iter()
        .enumerate()
        .map(|(index, (source_index, outcome))| {
            let filename = outcome.path.to_string_lossy();
            let stem = format!("{index:04}");
            fs_err::write(directory.join(format!("{stem}.path")), filename.as_bytes())?;
            fs_err::rename(
                stage.join(format!("{source_index}.ours")),
                directory.join(format!("{stem}.ours.marshal")),
            )?;
            fs_err::rename(
                stage.join(format!("{source_index}.cpython")),
                directory.join(format!("{stem}.cpython.marshal")),
            )
        })
        .collect::<Vec<Result<(), std::io::Error>>>()
        .into_iter()
        .collect()
}

fn discover(roots: &[PathBuf], extensions: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for root in roots {
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                entry.depth() == 0 || !entry.file_type().is_dir() || entry.file_name() != ".git"
            })
        {
            let entry = entry.map_err(|error| {
                format!("failed to walk corpus root {}: {error}", root.display())
            })?;
            if entry.file_type().is_file()
                && entry.path().extension().is_some_and(|extension| {
                    extension == "py"
                        || extensions
                            .iter()
                            .any(|expected| extension == expected.as_str())
                })
            {
                paths.push(entry.into_path());
            }
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Compares source bytes against a fresh reference process without normalizing marshal output.
fn compare(
    path: &Path,
    reference: &Path,
    options: &CompileOptions,
    retain_mismatch_bytes: bool,
) -> Outcome {
    let output = Command::new(reference)
        .args(["--optimize", &options.optimization_level.to_string()])
        .arg(path)
        .output();
    let oracle = match output {
        Ok(output) if output.status.success() => OracleOutcome::Compiled(output.stdout),
        Ok(output) if output.status.code() == Some(2) => {
            OracleOutcome::Rejected(one_line(&output.stderr))
        }
        Ok(output) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::OracleFailure,
                detail: format!(
                    "status {:?}: {}",
                    output.status.code(),
                    one_line(&output.stderr)
                ),
                oracle_rejected: false,
                mismatch_bytes: None,
            };
        }
        Err(error) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::OracleFailure,
                detail: error.to_string(),
                oracle_rejected: false,
                mismatch_bytes: None,
            };
        }
    };
    let oracle_rejected = matches!(&oracle, OracleOutcome::Rejected(_));
    let source = match fs_err::read(path) {
        Ok(source) => source,
        Err(error) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::OracleFailure,
                detail: error.to_string(),
                oracle_rejected,
                mismatch_bytes: None,
            };
        }
    };
    let compiled = compile_input(&source, &path.to_string_lossy(), options);
    classify_with_mismatch_bytes(path, oracle, compiled, retain_mismatch_bytes)
}

fn compile_input(source: &[u8], filename: &str, options: &CompileOptions) -> CompilerOutcome {
    capture_compiler(|| {
        compile_bytes_with_options(source, filename, options).map(|module| module.marshal())
    })
}

fn capture_compiler(compile: impl FnOnce() -> Result<Vec<u8>, CompileError>) -> CompilerOutcome {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(compile)) {
        Ok(Ok(marshal)) => CompilerOutcome::Compiled(marshal),
        Ok(Err(error @ (CompileError::InvalidFilename | CompileError::InvalidIdentifier(_)))) => {
            CompilerOutcome::Rejected(error.to_string())
        }
        Ok(Err(CompileError::Unsupported(message))) => CompilerOutcome::Unsupported(message),
        Ok(Err(CompileError::Parse(error))) => CompilerOutcome::Rejected(error.to_string()),
        Ok(Err(
            CompileError::InvalidPattern { message, .. }
            | CompileError::InvalidSyntax { message, .. }
            | CompileError::InvalidDocstring { message, .. },
        )) => CompilerOutcome::Rejected(message),
        Ok(Err(CompileError::Decode(error))) => CompilerOutcome::Rejected(error.to_string()),
        Ok(Err(CompileError::Internal(message))) => CompilerOutcome::Failed(message),
        Ok(Err(error)) => CompilerOutcome::Failed(error.to_string()),
        Err(payload) => CompilerOutcome::Failed(format!(
            "panic while compiling or marshaling: {}",
            panic_message(payload.as_ref())
        )),
    }
}

#[cfg(test)]
fn classify(path: &Path, oracle: OracleOutcome, compiled: CompilerOutcome) -> Outcome {
    classify_with_mismatch_bytes(path, oracle, compiled, false)
}

fn classify_with_mismatch_bytes(
    path: &Path,
    oracle: OracleOutcome,
    compiled: CompilerOutcome,
    retain_mismatch_bytes: bool,
) -> Outcome {
    let oracle_rejected = matches!(&oracle, OracleOutcome::Rejected(_));
    let mut mismatch_bytes = None;
    let (kind, detail) = match (oracle, compiled) {
        (OracleOutcome::Compiled(oracle), CompilerOutcome::Compiled(ours)) => {
            if ours == oracle {
                (OutcomeKind::Exact, format!("{} bytes", ours.len()))
            } else {
                let first_difference = ours
                    .iter()
                    .zip(&oracle)
                    .position(|(ours, oracle)| ours != oracle)
                    .unwrap_or_else(|| ours.len().min(oracle.len()));
                let ours_byte = ours.get(first_difference).copied();
                let oracle_byte = oracle.get(first_difference).copied();
                let detail = format!(
                    "first difference at byte {first_difference} ({ours_byte:?} != {oracle_byte:?}); ours={} bytes, CPython={} bytes",
                    ours.len(),
                    oracle.len()
                );
                if retain_mismatch_bytes {
                    mismatch_bytes = Some(MismatchBytes { ours, oracle });
                }
                (OutcomeKind::ByteMismatch, detail)
            }
        }
        (OracleOutcome::Compiled(_), CompilerOutcome::Unsupported(detail)) => {
            (OutcomeKind::Unsupported, detail)
        }
        (OracleOutcome::Compiled(_), CompilerOutcome::Rejected(detail)) => {
            (OutcomeKind::ParseMismatch, detail)
        }
        (OracleOutcome::Compiled(_), CompilerOutcome::Failed(detail)) => {
            (OutcomeKind::CompilerFailure, detail)
        }
        (OracleOutcome::Rejected(cpython), CompilerOutcome::Compiled(ours)) => (
            OutcomeKind::RejectionMismatch,
            format!(
                "CPython rejected ({cpython}); Serc compiled successfully ({} marshal bytes)",
                ours.len()
            ),
        ),
        (
            OracleOutcome::Rejected(cpython),
            CompilerOutcome::Unsupported(serc) | CompilerOutcome::Rejected(serc),
        ) => (
            OutcomeKind::MatchedRejection,
            format!("CPython rejected ({cpython}); Serc rejected ({serc})"),
        ),
        (OracleOutcome::Rejected(cpython), CompilerOutcome::Failed(serc)) => (
            OutcomeKind::CompilerFailure,
            format!("CPython rejected ({cpython}); Serc failed ({serc})"),
        ),
    };

    Outcome {
        path: path.to_path_buf(),
        kind,
        detail,
        oracle_rejected,
        mismatch_bytes,
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

fn one_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        CompileOptions, CompilerOutcome, Options, OracleOutcome, OutcomeKind, PythonVersion,
        ReferenceIdentity, capture_compiler, classify, compile_input, discover,
        validate_outcome_counts,
    };
    use clap::{Parser, error::ErrorKind};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    fn parse_options(args: &[&str]) -> Result<Options, clap::Error> {
        Options::try_parse_from(
            ["compare_cpython_corpus", "--reference", "reference"]
                .into_iter()
                .chain(args.iter().copied()),
        )
    }

    #[test]
    fn default_corpus_and_explicit_roots() {
        let options = parse_options(&[]).unwrap();
        assert_eq!(options.roots, super::default_roots());
        assert_eq!(options.python_version, PythonVersion::Py314);
        assert_eq!(options.optimize, serc::OptimizationLevel::Zero);
        assert_eq!(options.examples, 10);
        let options = parse_options(&["corpus"]).unwrap();
        assert_eq!(options.roots, [Path::new("corpus")]);
    }

    #[test]
    fn minimal_reference_is_required_and_optimization_is_validated() {
        let error = Options::try_parse_from(["compare_cpython_corpus"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
        let options = parse_options(&["--optimize", "2", "--require-all"]).unwrap();
        assert_eq!(options.reference, Path::new("reference"));
        assert_eq!(options.optimize, serc::OptimizationLevel::Two);
        assert!(options.require_all);
        assert!(parse_options(&["--optimize", "3"]).is_err());
    }

    #[test]
    fn minimal_reference_identity_checks_protocol_and_runtime() {
        let mut value = serde_json::json!({
            "protocol": "minimal-v2",
            "implementation": "cpython",
            "version": "3.14.5",
            "release_level": "final",
            "release_serial": 0,
            "magic": PythonVersion::Py314.target().magic_hex(),
        });
        let identity: super::ReferenceIdentity = serde_json::from_value(value.clone()).unwrap();
        assert!(identity.matches(PythonVersion::Py314));
        assert!(!identity.matches(PythonVersion::Py313));
        value["protocol"] = "minimal-v1".into();
        let identity: super::ReferenceIdentity = serde_json::from_value(value.clone()).unwrap();
        assert!(!identity.matches(PythonVersion::Py314));
        value.as_object_mut().unwrap().remove("protocol");
        assert!(serde_json::from_value::<super::ReferenceIdentity>(value).is_err());
    }

    #[test]
    fn minimal_reference_compares_exact_bytes_at_each_optimization_level() {
        for (version, variable) in [
            (PythonVersion::Py312, "SERC_TEST_CPYTHON_REFERENCE_312"),
            (PythonVersion::Py313, "SERC_TEST_CPYTHON_REFERENCE_313"),
            (PythonVersion::Py314, "SERC_TEST_CPYTHON_REFERENCE"),
            (PythonVersion::Py315, "SERC_TEST_CPYTHON_REFERENCE_315"),
        ] {
            let Some(reference) = std::env::var_os(variable) else {
                eprintln!("skipping {version} minimal reference test: {variable} is unset");
                continue;
            };
            assert_reference_matches(Path::new(&reference), version);
        }
    }

    /// Checks byte identity and source rejection under each reference's fixed startup.
    fn assert_reference_matches(reference: &Path, python_version: PythonVersion) {
        use serc::OptimizationLevel;

        super::verify_reference(reference, python_version).unwrap();
        let options = CompileOptions {
            python_version,
            ..CompileOptions::default()
        };
        let directory = tempfile::tempdir().unwrap();
        let sharing = concat!(
            "values = {'high', 'low', 'medium'}\n",
            "def second(x):\n    return x in {'high', 'low', 'medium'}\n",
            "f = lambda x: x in {('bootstrap_token_827',), ('secondary_827',)}\n",
            "x in {('bootstrap_token_827',), ('secondary_827',)}\n",
        );
        let optimized = concat!(
            "'module docstring'\nassert __debug__\n",
            "def f():\n    'function docstring'\n    assert False\n    return __debug__\n",
        );
        for (name, source) in [
            ("module_only.py", b"pass\n".as_slice()),
            ("sharing.py", sharing.as_bytes()),
            (
                "minimal_ast_startup.py",
                concat!(
                    "def first(value):\n    return value in {'left', 'marker_732'}, 1\n",
                    "def second(value):\n    return value in {'left', 'marker_732'}, 2\n",
                )
                .as_bytes(),
            ),
            (
                "minimal_signal_startup.py",
                concat!(
                    "def first(value):\n    return value in {'pause', 'marker_732'}, 1\n",
                    "def second(value):\n    return value in {'pause', 'marker_732'}, 2\n",
                )
                .as_bytes(),
            ),
            (
                "minimal_ast_identifier.py",
                concat!(
                    "def metadata(value):\n    return value.left\n",
                    "def first(value):\n    return value in {'left', 'marker_732'}, 1\n",
                    "def second(value):\n    return value in {'left', 'marker_732'}, 2\n",
                )
                .as_bytes(),
            ),
            (
                "minimal_signal_literal.py",
                concat!(
                    "def prior():\n    return 'pause'\n",
                    "def first(value):\n    return value in {'pause', 'marker_732'}, 1\n",
                    "def second(value):\n    return value in {'pause', 'marker_732'}, 2\n",
                )
                .as_bytes(),
            ),
            ("optimized.py", optimized.as_bytes()),
            ("latin1.py", b"# coding: latin-1\nvalue = 'caf\xe9'\n"),
        ] {
            let path = directory.path().join(name);
            fs::write(&path, source).unwrap();
            for optimization_level in [
                OptimizationLevel::Zero,
                OptimizationLevel::One,
                OptimizationLevel::Two,
            ] {
                let options = CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                };
                let outcome = super::compare(&path, reference, &options, true);
                assert_eq!(
                    outcome.kind,
                    OutcomeKind::Exact,
                    "{python_version} {name} -O{optimization_level}: {}",
                    outcome.detail
                );

                let mut command = std::process::Command::new(reference);
                command
                    .args(["--optimize", &optimization_level.to_string()])
                    .arg(&path);
                let output = command
                    .current_dir(directory.path())
                    .env("PYTHONHOME", "/unused-python-home")
                    .env("PYTHONPATH", directory.path())
                    .env("PYTHONOPTIMIZE", "2")
                    .env("PYTHONHASHSEED", "random")
                    .env("PYTHONUTF8", "0")
                    .output()
                    .unwrap();
                assert!(output.status.success(), "{output:?}");
                let compiled = compile_input(source, path.to_str().unwrap(), &options);
                let outcome = classify(&path, OracleOutcome::Compiled(output.stdout), compiled);
                assert_eq!(
                    outcome.kind,
                    OutcomeKind::Exact,
                    "noisy {python_version} {name} -O{optimization_level}: {}",
                    outcome.detail
                );
            }
        }
        for (name, source) in [
            ("syntax.py", b"if:\n".as_slice()),
            ("nul.py", b"value = 1\0\n"),
            ("semantic.py", b"return\n"),
            ("surrogate_docstring.py", b"def f():\n    '\\ud800'\n"),
            ("encoding.py", b"# coding: invalid-encoding\npass\n"),
        ] {
            let path = directory.path().join(name);
            fs::write(&path, source).unwrap();
            let outcome = super::compare(&path, reference, &options, false);
            // Python 3.12 accepts surrogate docstrings; later versions reject them.
            let expected =
                if python_version == PythonVersion::Py312 && name == "surrogate_docstring.py" {
                    OutcomeKind::Exact
                } else {
                    OutcomeKind::MatchedRejection
                };
            assert_eq!(
                outcome.kind, expected,
                "{python_version} {name}: {}",
                outcome.detail
            );
        }
        let outcome = super::compare(
            &directory.path().join("missing.py"),
            reference,
            &options,
            false,
        );
        assert_eq!(
            outcome.kind,
            OutcomeKind::OracleFailure,
            "{}",
            outcome.detail
        );
    }

    #[test]
    fn marshal_sharing_differences_remain_byte_mismatches() {
        let source = "f = lambda x: x in {'profile_specific_identifier', 'z'}\nx in {'profile_specific_identifier', 'z'}\n";
        let state = serc::InitialInternState::new(["profile_specific_identifier"]);
        let ordinary = serc::compile(source, "sharing.py").unwrap().marshal();
        let interned = serc::compile_with_intern_state(source, "sharing.py", &state)
            .unwrap()
            .marshal();
        let outcome = classify(
            Path::new("sharing.py"),
            OracleOutcome::Compiled(ordinary),
            CompilerOutcome::Compiled(interned),
        );
        assert_eq!(outcome.kind, OutcomeKind::ByteMismatch);
    }

    #[test]
    fn selected_python_version_is_parsed_and_validated() {
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let options = parse_options(&["--python-version", &version.to_string()]).unwrap();
            assert_eq!(options.python_version, version);
        }
        let error = parse_options(&["--python-version", "3.11"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn rejects_non_bare_extensions() {
        for extension in ["", ".pyi", "nested/pyi"] {
            let error = parse_options(&["--extension", extension]).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::ValueValidation);
        }
    }

    #[test]
    fn expected_counts_reject_duplicate_kinds_and_exact_alias() {
        for args in [
            [
                "--expected-outcome",
                "exact=2",
                "--expected-outcome",
                "exact=2",
            ],
            ["--expected-outcome", "exact=2", "--expected-exact", "2"],
            ["--expected-exact", "2", "--expected-outcome", "exact=2"],
        ] {
            let options = parse_options(&args).unwrap();
            assert!(
                options
                    .expected_outcome_counts()
                    .unwrap_err()
                    .contains("may only be specified once")
            );
        }
    }

    #[test]
    fn frozen_counts_are_preserved() {
        let options = parse_options(&[
            "--expected-files",
            "3",
            "--expected-exact",
            "2",
            "--expected-cpython-rejected",
            "1",
            "--expected-outcome",
            "matched-rejection=1",
            "--expected-outcome",
            "byte-mismatch=0",
            "--require-all",
            "--require-rejection-parity",
            "--examples",
            "0",
            "corpus",
        ])
        .unwrap();
        assert!(options.require_all && options.require_rejection_parity);
        assert_eq!(options.expected_files, Some(3));
        assert_eq!(options.expected_cpython_rejected, Some(1));
        assert_eq!(options.examples, 0);
        assert_eq!(
            options.expected_outcome_counts().unwrap(),
            BTreeMap::from([
                (OutcomeKind::Exact, 2),
                (OutcomeKind::MatchedRejection, 1),
                (OutcomeKind::ByteMismatch, 0),
            ])
        );
    }

    #[test]
    fn oracle_identity_rejects_other_targets_and_release_candidates() {
        for python_version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let target = python_version.target();
            let (release_level, release_serial) = match python_version {
                PythonVersion::Py312 | PythonVersion::Py313 | PythonVersion::Py314 => ("final", 0),
                PythonVersion::Py315 => ("candidate", 2),
            };
            let mut identity = ReferenceIdentity {
                protocol: "minimal-v2".to_string(),
                implementation: target.implementation.to_string(),
                version: target.version_string(),
                release_level: release_level.to_string(),
                release_serial,
                magic: target.magic_hex(),
            };
            assert!(identity.matches(python_version));
            for other in [
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ] {
                assert_eq!(identity.matches(other), other == python_version);
            }
            identity.release_serial += 1;
            assert!(!identity.matches(python_version));
            identity.release_serial = release_serial;
            identity.release_level = "alpha".to_string();
            assert!(!identity.matches(python_version));
            identity.release_level = release_level.to_string();
            identity.magic = "wrong magic".to_string();
            assert!(!identity.matches(python_version));
        }
    }

    #[test]
    fn selected_target_reaches_byte_input() {
        for python_version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let options = CompileOptions {
                python_version,
                ..CompileOptions::default()
            };
            let module =
                serc::compile_bytes_with_options(b"value = 1\n", "target.py", &options).unwrap();
            assert_eq!(module.python_version(), python_version);
            assert_eq!(module.target(), python_version.target());
        }
    }

    #[test]
    fn byte_input_consumes_exactly_one_byte_order_mark() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bom.py");
        let filename = path.to_str().unwrap();
        for (source, expected) in [
            ("value = '\u{feff}'\n", Some("value = '\u{feff}'\n")),
            ("\u{feff}value = 1\n", Some("value = 1\n")),
            ("\u{feff}\u{feff}value = 1\n", None),
        ] {
            fs::write(&path, source).unwrap();
            let compiled = compile_input(
                &fs::read(&path).unwrap(),
                filename,
                &CompileOptions::default(),
            );
            if let Some(expected) = expected {
                let expected = serc::compile(expected, filename).unwrap().marshal();
                let CompilerOutcome::Compiled(bytes) = compiled else {
                    panic!("a single BOM must be accepted");
                };
                assert_eq!(bytes, expected);
            } else {
                assert!(matches!(compiled, CompilerOutcome::Rejected(_)));
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn mismatch_dumps_preserve_the_original_comparison() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let oracle = directory.path().join("oracle");
        let path = directory.path().join("source.py");
        let second_path = directory.path().join("second.py");
        let source = "f = lambda x: x in {'profile_specific_identifier', 'z'}\nx in {'profile_specific_identifier', 'z'}\n";
        let state = serc::InitialInternState::new(["profile_specific_identifier"]);
        let expected = serc::compile_with_intern_state(source, path.to_str().unwrap(), &state)
            .unwrap()
            .marshal();
        let second_expected =
            serc::compile_with_intern_state(source, second_path.to_str().unwrap(), &state)
                .unwrap()
                .marshal();
        fs::write(&path, source).unwrap();
        fs::write(&second_path, source).unwrap();
        fs::write(&oracle, "#!/bin/sh\nprintf 'oracle bytes'\n").unwrap();
        fs::set_permissions(&oracle, fs::Permissions::from_mode(0o700)).unwrap();
        let options = CompileOptions {
            initial_intern_state: state.clone(),
            ..CompileOptions::default()
        };
        let without_dump = super::compare(&path, &oracle, &options, false);
        assert_eq!(
            without_dump.kind,
            OutcomeKind::ByteMismatch,
            "first compare: {}",
            without_dump.detail
        );
        assert!(without_dump.mismatch_bytes.is_none());
        let mut outcome = super::compare(&path, &oracle, &options, true);
        assert_eq!(
            outcome.kind,
            OutcomeKind::ByteMismatch,
            "retained compare: {}",
            outcome.detail
        );
        fs::write(&oracle, "#!/bin/sh\nprintf 'other oracle bytes'\n").unwrap();
        let mut second_outcome = super::compare(&second_path, &oracle, &options, true);
        assert_eq!(
            second_outcome.kind,
            OutcomeKind::ByteMismatch,
            "second compare: {}",
            second_outcome.detail
        );

        let dump = directory.path().join("dump");
        fs::create_dir_all(&dump).unwrap();
        let stage = tempfile::tempdir_in(&dump).unwrap();
        super::stage_mismatch(&mut outcome, stage.path(), 0).unwrap();
        super::stage_mismatch(&mut second_outcome, stage.path(), 2).unwrap();
        assert!(outcome.mismatch_bytes.is_none());
        assert!(second_outcome.mismatch_bytes.is_none());

        // Neither the source nor a subsequent oracle invocation may replace
        // the bytes that produced the reported mismatch.
        fs::write(&path, "changed = True\n").unwrap();
        fs::write(&second_path, "changed = True\n").unwrap();
        fs::write(&oracle, "#!/bin/sh\nexit 1\n").unwrap();
        let exact = super::Outcome {
            path: path.clone(),
            kind: OutcomeKind::Exact,
            detail: String::new(),
            oracle_rejected: false,
            mismatch_bytes: None,
        };
        super::dump_mismatches(&[outcome, exact, second_outcome], stage.path(), &dump).unwrap();
        assert_eq!(fs::read(dump.join("0000.ours.marshal")).unwrap(), expected);
        assert_eq!(
            fs::read(dump.join("0000.cpython.marshal")).unwrap(),
            b"oracle bytes"
        );
        assert_eq!(
            fs::read(dump.join("0001.ours.marshal")).unwrap(),
            second_expected
        );
        assert_eq!(
            fs::read(dump.join("0001.cpython.marshal")).unwrap(),
            b"other oracle bytes"
        );
        assert_eq!(
            fs::read_to_string(dump.join("0000.path")).unwrap(),
            path.to_string_lossy()
        );
        assert_eq!(
            fs::read_to_string(dump.join("0001.path")).unwrap(),
            second_path.to_string_lossy()
        );
    }

    #[test]
    fn mismatch_staging_reports_io_errors() {
        let directory = tempfile::tempdir().unwrap();
        let not_a_directory = directory.path().join("file");
        fs::write(&not_a_directory, b"not a directory").unwrap();
        let mut outcome = super::classify_with_mismatch_bytes(
            Path::new("source.py"),
            super::OracleOutcome::Compiled(b"oracle".to_vec()),
            super::CompilerOutcome::Compiled(b"serc".to_vec()),
            true,
        );
        assert_eq!(outcome.kind, OutcomeKind::ByteMismatch);
        assert!(super::stage_mismatch(&mut outcome, &not_a_directory, 0).is_err());
    }

    #[test]
    fn require_all_rejects_a_corpus_without_exact_matches() {
        let counts = BTreeMap::from([(OutcomeKind::MatchedRejection, 3)]);

        let failures =
            validate_outcome_counts(&counts, 3, 0, 0, &BTreeMap::new(), None, true, false);

        assert!(
            failures
                .iter()
                .any(|failure| failure == "no files matched CPython exactly")
        );
    }

    #[test]
    fn cpython_rejection_requires_serc_rejection() {
        let compiled = compile_input(b"if:\n", "invalid.py", &CompileOptions::default());
        let outcome = classify(
            Path::new("invalid.py"),
            OracleOutcome::Rejected("SyntaxError: invalid syntax".to_string()),
            compiled,
        );

        assert_eq!(outcome.kind, OutcomeKind::MatchedRejection);
        assert!(outcome.oracle_rejected);
        assert!(outcome.detail.contains("Serc rejected"));
    }

    #[test]
    fn cpython_semantic_rejection_requires_serc_rejection() {
        let compiled = compile_input(b"return\n", "invalid.py", &CompileOptions::default());
        let outcome = classify(
            Path::new("invalid.py"),
            OracleOutcome::Rejected("SyntaxError: 'return' outside function".to_string()),
            compiled,
        );

        assert_eq!(outcome.kind, OutcomeKind::MatchedRejection);
        assert!(outcome.oracle_rejected);
        assert!(outcome.detail.contains("Serc rejected"));
    }

    #[test]
    fn cpython_rejection_detects_serc_acceptance() {
        let compiled = compile_input(b"pass\n", "valid.py", &CompileOptions::default());
        let outcome = classify(
            Path::new("valid.py"),
            OracleOutcome::Rejected("SyntaxError: invalid syntax".to_string()),
            compiled,
        );

        assert_eq!(outcome.kind, OutcomeKind::RejectionMismatch);
        assert!(outcome.kind.is_failure());
        assert!(outcome.oracle_rejected);
        assert!(outcome.detail.contains("Serc compiled successfully"));
    }

    #[test]
    fn cpython_rejected_count_is_independent_from_rejection_parity() {
        let counts = BTreeMap::from([(OutcomeKind::Exact, 1), (OutcomeKind::RejectionMismatch, 1)]);
        let expected = BTreeMap::from([(OutcomeKind::Exact, 1)]);

        assert!(
            validate_outcome_counts(&counts, 1, 0, 1, &expected, Some(1), true, false).is_empty()
        );
        assert_eq!(
            validate_outcome_counts(&counts, 1, 0, 1, &expected, Some(1), true, true),
            ["1 CPython-rejected files were not rejected cleanly by Serc"]
        );
    }

    #[test]
    fn surrogate_docstring_errors_count_as_matched_rejections() {
        let source = b"def f():\n    '\\ud800'\n";
        let filename = "surrogate_docstring.py";
        let compiled = compile_input(source, filename, &CompileOptions::default());
        let outcome = classify(
            Path::new(filename),
            OracleOutcome::Rejected("UnicodeEncodeError: surrogates not allowed".to_string()),
            compiled,
        );
        assert_eq!(outcome.kind, OutcomeKind::MatchedRejection);
        assert!(outcome.oracle_rejected);
    }

    #[test]
    fn compiler_panics_preserve_their_diagnostic() {
        let compiled = capture_compiler(|| -> Result<Vec<u8>, serc::CompileError> {
            panic!("sentinel compiler panic")
        });

        let CompilerOutcome::Failed(detail) = compiled else {
            panic!("a compiler panic must be classified as a failure");
        };
        assert!(detail.contains("sentinel compiler panic"));
    }

    #[test]
    fn compiler_failure_does_not_hide_cpython_rejection() {
        let outcome = classify(
            Path::new("invalid.py"),
            OracleOutcome::Rejected("SyntaxError: invalid syntax".to_string()),
            CompilerOutcome::Failed("sentinel failure".to_string()),
        );

        assert_eq!(outcome.kind, OutcomeKind::CompilerFailure);
        assert!(outcome.oracle_rejected);
        assert!(outcome.detail.contains("CPython rejected"));
        assert!(outcome.detail.contains("sentinel failure"));
    }

    #[test]
    fn pyi_discovery_is_opt_in() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("module.py"), "pass\n").unwrap();
        fs::write(
            directory.path().join("module.pyi"),
            "def function() -> None: ...\n",
        )
        .unwrap();
        fs::write(directory.path().join("notes.txt"), "not Python\n").unwrap();
        let roots = [directory.path().to_path_buf()];

        let options = parse_options(&[]).unwrap();
        let default = discover(&roots, &options.extensions).unwrap();
        assert_eq!(default, vec![directory.path().join("module.py")]);

        let options = parse_options(&["--extension", "pyi"]).unwrap();
        let with_stubs = discover(&roots, &options.extensions).unwrap();
        assert_eq!(
            with_stubs,
            vec![
                directory.path().join("module.py"),
                directory.path().join("module.pyi")
            ]
        );
    }

    #[test]
    fn discovery_excludes_git_metadata_directories() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join(".git/hooks")).unwrap();
        fs::create_dir_all(directory.path().join("nested/.git/hooks")).unwrap();
        fs::create_dir_all(directory.path().join(".github")).unwrap();
        fs::write(directory.path().join(".git/hooks/helper.py"), "pass\n").unwrap();
        fs::write(
            directory.path().join("nested/.git/hooks/helper.pyi"),
            "pass\n",
        )
        .unwrap();
        fs::write(directory.path().join(".github/script.py"), "pass\n").unwrap();
        fs::write(directory.path().join("module.py"), "pass\n").unwrap();
        fs::write(directory.path().join("nested/module.pyi"), "pass\n").unwrap();

        let roots = [directory.path().to_path_buf()];
        assert_eq!(
            discover(&roots, &["pyi".to_string()]).unwrap(),
            vec![
                directory.path().join(".github/script.py"),
                directory.path().join("module.py"),
                directory.path().join("nested/module.pyi"),
            ]
        );
    }

    #[test]
    fn validates_expected_outcome_counts() {
        let counts = BTreeMap::from([(OutcomeKind::Exact, 2), (OutcomeKind::MatchedRejection, 1)]);
        let expected =
            BTreeMap::from([(OutcomeKind::Exact, 2), (OutcomeKind::MatchedRejection, 1)]);

        assert!(
            validate_outcome_counts(&counts, 1, 0, 0, &expected, Some(1), true, false).is_empty()
        );
        let expected = BTreeMap::from([(OutcomeKind::Exact, 3), (OutcomeKind::ByteMismatch, 4)]);
        assert_eq!(
            validate_outcome_counts(&counts, 1, 0, 0, &expected, Some(2), false, false),
            [
                "found 2 exact matches, but expected 3",
                "found 0 byte mismatch outcomes, but expected 4",
                "CPython rejected 1 files, but expected 2",
            ]
        );
    }
}
