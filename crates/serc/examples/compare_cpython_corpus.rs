#![allow(clippy::print_stderr, clippy::print_stdout)]

use std::any::Any;
use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

use clap::Parser;
use serc::{
    CompileError, CompileOptions, CompiledModule, PythonVersion, compile_bytes_with_options,
    compile_with_options,
};

#[path = "support/caller_intern_model.rs"]
mod caller_intern_model;
#[path = "support/initial_intern_profile.rs"]
mod initial_intern_profile;
#[path = "support/native_output_cache.rs"]
mod native_output_cache;
#[path = "support/native_output_reader.rs"]
mod native_output_reader;
use rayon::prelude::*;
use walkdir::WalkDir;

const WORKER_STACK_SIZE: usize = 32 * 1024 * 1024;

const ORACLE: &str = r#"
import marshal
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
source = path.read_bytes()
try:
    code = compile(source, str(path), "exec", dont_inherit=True, optimize=0)
except Exception as error:
    print(f"{type(error).__name__}: {error}", file=sys.stderr)
    raise SystemExit(2)
sys.stdout.buffer.write(marshal.dumps(code))
"#;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum OutcomeKind {
    Exact,
    ByteMismatch,
    Unsupported,
    ParseMismatch,
    CompilerFailure,
    UnresolvedInternState,
    MatchedRejection,
    RejectionMismatch,
    NonUtf8,
    OracleFailure,
}

impl OutcomeKind {
    const ALL: [Self; 10] = [
        Self::Exact,
        Self::ByteMismatch,
        Self::Unsupported,
        Self::ParseMismatch,
        Self::CompilerFailure,
        Self::UnresolvedInternState,
        Self::MatchedRejection,
        Self::RejectionMismatch,
        Self::NonUtf8,
        Self::OracleFailure,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::ByteMismatch => "byte mismatch",
            Self::Unsupported => "unsupported",
            Self::ParseMismatch => "parse/decode mismatch",
            Self::CompilerFailure => "compiler failure",
            Self::UnresolvedInternState => "unresolved intern state",
            Self::MatchedRejection => "matched rejection",
            Self::RejectionMismatch => "rejection mismatch",
            Self::NonUtf8 => "non-UTF-8",
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
            Self::UnresolvedInternState => "unresolved-intern-state",
            Self::MatchedRejection => "matched-rejection",
            Self::RejectionMismatch => "rejection-mismatch",
            Self::NonUtf8 => "non-utf8",
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

enum CompilerInput {
    Bytes(Vec<u8>),
    Text(String),
}

enum OracleOutcome {
    Compiled(Vec<u8>),
    Rejected(String),
}

enum CompilerOutcome {
    Compiled(Vec<u8>),
    Unsupported(String),
    UnresolvedInternState(String),
    Rejected(String),
    Failed(String),
}

#[derive(Debug, Parser)]
#[command(about = "Compare a Python corpus against the pinned CPython target")]
struct Options {
    #[arg(long, default_value = "python3", value_name = "PATH")]
    python: String,
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
    #[arg(long)]
    require_all: bool,
    #[arg(long)]
    require_rejection_parity: bool,
    #[arg(long, default_value_t = 10)]
    examples: usize,
    #[arg(long, value_name = "DIR")]
    dump_mismatches: Option<PathBuf>,
    #[arg(long)]
    source_bytes: bool,
    #[arg(long, value_name = "PROFILE.json")]
    initial_intern_state: Option<PathBuf>,
    /// Diagnostic only: substitute the compiler's neutral startup model for caller state.
    #[arg(long, conflicts_with_all = ["initial_intern_state", "caller_intern_model"])]
    diagnostic_assume_startup_intern_state: bool,
    /// Reproduce a validated caller protocol from an independent pre-path baseline.
    #[arg(long, value_name = "MODEL.json")]
    caller_intern_model: Option<PathBuf>,
    /// Diagnostic only: preserve known caller entries and assume every other entry absent.
    #[arg(long, requires = "caller_intern_model")]
    caller_intern_assume_absent: bool,
}

impl Options {
    fn validate_intern_contract(&self, cache_read: bool, cache_write: bool) -> Result<(), String> {
        if self.diagnostic_assume_startup_intern_state {
            if cache_read
                || cache_write
                || self.require_all
                || self.require_rejection_parity
                || self.expected_files.is_some()
                || self.expected_exact.is_some()
                || self.expected_cpython_rejected.is_some()
                || !self.expected_outcomes.is_empty()
            {
                return Err("diagnostic startup assumptions cannot validate a compatibility gate or native cache".to_string());
            }
        } else if self.initial_intern_state.is_none() {
            return Err("comparison requires --initial-intern-state with provenance matching this oracle; use --diagnostic-assume-startup-intern-state only for exploratory comparisons".to_string());
        }
        Ok(())
    }

    fn validate_assume_absent(&self, cache_read: bool) -> Result<(), String> {
        if self.caller_intern_assume_absent && !cache_read {
            Err("--caller-intern-assume-absent requires admitted native cache reading".to_string())
        } else {
            Ok(())
        }
    }

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
    if options.caller_intern_model.is_some()
        && (!options.source_bytes
            || options.initial_intern_state.is_none()
            || env::var_os("SERC_NATIVE_CACHE_WRITE").is_some())
    {
        eprintln!(
            "caller modeling requires byte sources and a native provenance profile, and cannot capture new native output"
        );
        return ExitCode::from(2);
    }
    let read_config = env::var_os("SERC_NATIVE_CACHE_READ");
    if let Err(message) = options.validate_intern_contract(
        read_config.is_some(),
        env::var_os("SERC_NATIVE_CACHE_WRITE").is_some(),
    ) {
        eprintln!("{message}");
        return ExitCode::from(2);
    }
    if options.diagnostic_assume_startup_intern_state {
        eprintln!(
            "DIAGNOSTIC ONLY: assuming neutral startup intern state for the pathlib oracle; output equality is conditional on this unverified assumption"
        );
    }
    if let Err(message) = options.validate_assume_absent(read_config.is_some()) {
        eprintln!("{message}");
        return ExitCode::from(2);
    }
    if read_config.is_some()
        && (env::var_os("SERC_NATIVE_CACHE_WRITE").is_some()
            || options.dump_mismatches.is_some()
            || !options.source_bytes
            || options.initial_intern_state.is_none())
    {
        eprintln!(
            "native cache reading requires byte sources and an intern profile, and cannot combine writing or native mismatch dumping"
        );
        return ExitCode::from(2);
    }
    if let Err(message) = verify_python(&options.python, options.python_version) {
        eprintln!("{message}");
        return ExitCode::from(2);
    }
    let profile = match options
        .initial_intern_state
        .as_deref()
        .map(|path| {
            initial_intern_profile::load(
                path,
                &options.python,
                options.python_version,
                options.source_bytes,
                ORACLE,
            )
        })
        .transpose()
    {
        Ok(profile) => profile,
        Err(error) => {
            eprintln!("invalid initial intern-state profile: {error}");
            return ExitCode::from(2);
        }
    };
    if let Some(profile) = &profile {
        let role = if options.caller_intern_model.is_some() {
            "Native provenance snapshot"
        } else {
            "Partial caller intern-state evidence"
        };
        println!(
            "{role}: {} ({} observed entries; {} guaranteed retained entries)",
            profile.snapshot_sha256, profile.entries, profile.retained_entries
        );
    }
    let caller_model = match options
        .caller_intern_model
        .as_deref()
        .map(|path| {
            caller_intern_model::load(
                path,
                options
                    .initial_intern_state
                    .as_deref()
                    .expect("caller profile checked"),
                options.python_version,
                options.source_bytes,
                ORACLE,
                options.caller_intern_assume_absent,
            )
        })
        .transpose()
    {
        Ok(model) => model,
        Err(error) => {
            eprintln!("invalid caller intern-state model: {error}");
            return ExitCode::from(2);
        }
    };
    let compile_options = CompileOptions {
        python_version: options.python_version,
        initial_intern_state: if let Some(profile) = profile {
            profile.state
        } else {
            assert!(options.diagnostic_assume_startup_intern_state);
            serc::InitialInternState::default()
        },
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
    if caller_model.is_some() {
        for path in &paths {
            if let Err(error) = caller_intern_model::validate_filename(path) {
                eprintln!(
                    "unsupported caller-model filename {}: {error}",
                    path.display()
                );
                return ExitCode::from(2);
            }
        }
    }

    let cache = match env::var_os("SERC_NATIVE_CACHE_WRITE") {
        Some(config) => {
            let Some(profile) = options
                .initial_intern_state
                .as_deref()
                .filter(|_| options.source_bytes)
            else {
                eprintln!(
                    "native output capture requires --source-bytes and --initial-intern-state"
                );
                return ExitCode::from(2);
            };
            match native_output_cache::Writer::new(
                Path::new(&config),
                profile,
                &options.python,
                ORACLE,
                &paths,
            ) {
                Ok(writer) => Some(writer),
                Err(error) => {
                    eprintln!("failed to initialize native output capture: {error}");
                    return ExitCode::FAILURE;
                }
            }
        }
        None => None,
    };

    let reader = match read_config {
        Some(config) => match native_output_reader::Reader::new(
            Path::new(&config),
            options
                .initial_intern_state
                .as_deref()
                .expect("reader profile checked"),
            &options.python,
            ORACLE,
            options.python_version,
            &paths,
            caller_model.as_ref(),
        ) {
            Ok(reader) => {
                println!(
                    "Native oracle source: admitted cached bytes; every candidate is freshly compiled"
                );
                Some(reader)
            }
            Err(error) => {
                eprintln!("failed to open admitted native cache: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };

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
                if let Some(reader) = &reader {
                    return Ok(reader.compare(
                        index,
                        path,
                        &compile_options,
                        caller_model.as_ref(),
                    ));
                }
                let mut observation = cache.as_ref().map(|writer| writer.begin(index, path));
                let mut outcome = compare(
                    path,
                    &options.python,
                    options.source_bytes,
                    &compile_options,
                    caller_model.as_ref(),
                    observation.as_mut(),
                    options.dump_mismatches.is_some(),
                );
                if let Some(observation) = observation {
                    observation.finish(&outcome);
                }
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
        options.python,
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
        OutcomeKind::UnresolvedInternState,
        OutcomeKind::NonUtf8,
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
        OutcomeKind::UnresolvedInternState,
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
    if let Some(reader) = reader {
        if let Err(error) = reader.finish(gate_failures.is_empty()) {
            eprintln!("admitted native cache comparison failed: {error}");
            return ExitCode::FAILURE;
        }
    }
    if let Some(cache) = cache {
        if let Err(error) = cache.finish(gate_failures.is_empty()) {
            eprintln!("native output capture failed: {error}");
            return ExitCode::FAILURE;
        }
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
    let unresolved = count(OutcomeKind::UnresolvedInternState);
    if unresolved != 0 {
        failures.push(format!(
            "{unresolved} files require unresolved caller intern-state evidence"
        ));
    }

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
struct PythonIdentity {
    implementation: String,
    version: String,
    release_level: String,
    release_serial: u8,
    magic: String,
}

impl PythonIdentity {
    fn matches(&self, version: PythonVersion) -> bool {
        let target = version.target();
        let (release_level, release_serial) = match version {
            PythonVersion::Py312 | PythonVersion::Py313 | PythonVersion::Py314 => ("final", 0),
            PythonVersion::Py315 => ("candidate", 2),
        };
        self.implementation == target.implementation
            && self.version == target.version_string()
            && self.release_level == release_level
            && self.release_serial == release_serial
            && self.magic == target.magic_hex()
    }
}

fn verify_python(python: &str, python_version: PythonVersion) -> Result<(), String> {
    let target = python_version.target();
    let output = Command::new(python)
        .args([
            "-B",
            "-I",
            "-S",
            "-c",
            "import importlib.util, json, sys; print(json.dumps(dict(implementation=sys.implementation.name, version='.'.join(map(str, sys.version_info[:3])), release_level=sys.version_info.releaselevel, release_serial=sys.version_info.serial, magic=importlib.util.MAGIC_NUMBER.hex())))",
        ])
        .output()
        .map_err(|error| format!("failed to run {python}: {error}"))?;
    let identity: PythonIdentity = serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "failed to read {python} identity: {error}; {}",
            one_line(&output.stderr)
        )
    })?;
    if output.status.success() && identity.matches(python_version) {
        Ok(())
    } else {
        Err(format!(
            "{python} is {identity:?}; the compiler and oracle require {} {} with magic {}",
            target.implementation,
            target.tag,
            target.magic_hex()
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

fn compare(
    path: &Path,
    python: &str,
    source_bytes: bool,
    options: &CompileOptions,
    caller_model: Option<&caller_intern_model::LoadedModel>,
    mut observation: Option<&mut native_output_cache::Observation<'_>>,
    retain_mismatch_bytes: bool,
) -> Outcome {
    let output = Command::new(python)
        .args(["-I", "-S", "-c", ORACLE])
        .arg(path)
        .output();
    if let Some(observation) = &mut observation {
        observation.native(&output);
    }
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

    let source = if source_bytes {
        fs_err::read(path).map(CompilerInput::Bytes)
    } else {
        read_utf8_file(path).map(CompilerInput::Text)
    };
    let source = match source {
        Ok(source) => source,
        Err(error) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: if error.kind() == std::io::ErrorKind::InvalidData {
                    OutcomeKind::NonUtf8
                } else {
                    OutcomeKind::OracleFailure
                },
                detail: error.to_string(),
                oracle_rejected,
                mismatch_bytes: None,
            };
        }
    };

    let filename = path.to_string_lossy();
    let compiled = compile_input(&source, &filename, options, caller_model);
    if let Some(observation) = observation {
        observation.compiled(&source, &compiled);
    }

    classify_with_mismatch_bytes(path, oracle, compiled, retain_mismatch_bytes)
}

/// Consume an optional file encoding signature before passing decoded text to the compiler.
fn read_utf8_file(path: &Path) -> std::io::Result<String> {
    let mut source = fs_err::read_to_string(path)?;
    if source.starts_with('\u{feff}') {
        source.replace_range(..'\u{feff}'.len_utf8(), "");
    }
    Ok(source)
}

fn compile_module_input(
    source: &CompilerInput,
    filename: &str,
    options: &CompileOptions,
    caller_model: Option<&caller_intern_model::LoadedModel>,
) -> Result<CompiledModule, CompileError> {
    // Derive the per-input mask from the same in-memory bytes that the cache
    // reader hashes and this invocation compiles. Never reread for the model.
    let file_options = match (caller_model, source) {
        (Some(model), CompilerInput::Bytes(bytes)) => {
            Some(model.options_for(filename, bytes, options))
        }
        (Some(_), CompilerInput::Text(_)) => {
            return Err(CompileError::Internal(
                "caller model requires source-bytes input".to_string(),
            ));
        }
        (None, _) => None,
    };
    let options = file_options.as_ref().unwrap_or(options);
    match source {
        CompilerInput::Bytes(source) => compile_bytes_with_options(source, filename, options),
        CompilerInput::Text(source) => compile_with_options(source, filename, options),
    }
}

fn compile_input(
    source: &CompilerInput,
    filename: &str,
    options: &CompileOptions,
    caller_model: Option<&caller_intern_model::LoadedModel>,
) -> CompilerOutcome {
    capture_compiler(|| {
        compile_module_input(source, filename, options, caller_model).map(|module| module.marshal())
    })
}

fn capture_compiler(compile: impl FnOnce() -> Result<Vec<u8>, CompileError>) -> CompilerOutcome {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(compile)) {
        Ok(Ok(marshal)) => CompilerOutcome::Compiled(marshal),
        Ok(Err(error @ CompileError::InvalidFilename)) => {
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
        Ok(Err(error @ CompileError::UnresolvedInternState { .. })) => {
            CompilerOutcome::UnresolvedInternState(error.to_string())
        }
        Ok(Err(CompileError::Internal(message))) => CompilerOutcome::Failed(message),
        Ok(Err(error)) => CompilerOutcome::Failed(error.to_string()),
        Err(payload) => CompilerOutcome::Failed(format!(
            "panic while compiling or marshaling: {}",
            panic_message(payload.as_ref())
        )),
    }
}

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
        (_, CompilerOutcome::UnresolvedInternState(detail)) => {
            (OutcomeKind::UnresolvedInternState, detail)
        }
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
        CompileOptions, CompilerInput, CompilerOutcome, ORACLE, Options, OracleOutcome,
        OutcomeKind, PythonIdentity, PythonVersion, capture_compiler, classify, compile_input,
        compile_module_input, discover, validate_outcome_counts, verify_python,
    };
    use clap::{Parser, error::ErrorKind};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn default_corpus_and_explicit_roots() {
        let options = Options::try_parse_from(["compare_cpython_corpus"]).unwrap();
        assert_eq!(options.roots, super::default_roots());
        assert_eq!(options.python, "python3");
        assert_eq!(options.python_version, PythonVersion::Py314);
        assert_eq!(options.examples, 10);
        let options = Options::try_parse_from(["compare_cpython_corpus", "corpus"]).unwrap();
        assert_eq!(options.roots, [Path::new("corpus")]);
    }

    #[test]
    fn assume_absent_is_opt_in_and_requires_a_model_and_cached_native_outputs() {
        let defaults = Options::try_parse_from(["compare_cpython_corpus"]).unwrap();
        assert!(!defaults.caller_intern_assume_absent);
        assert!(defaults.validate_assume_absent(false).is_ok());
        let error =
            Options::try_parse_from(["compare_cpython_corpus", "--caller-intern-assume-absent"])
                .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
        let diagnostic = Options::try_parse_from([
            "compare_cpython_corpus",
            "--caller-intern-model",
            "model.json",
            "--caller-intern-assume-absent",
        ])
        .unwrap();
        assert!(diagnostic.validate_assume_absent(false).is_err());
        assert!(diagnostic.validate_assume_absent(true).is_ok());
    }

    #[test]
    fn comparison_requires_caller_evidence_and_diagnostics_cannot_gate() {
        let defaults = Options::try_parse_from(["compare_cpython_corpus"]).unwrap();
        assert!(defaults.validate_intern_contract(false, false).is_err());
        let profiled = Options::try_parse_from([
            "compare_cpython_corpus",
            "--initial-intern-state",
            "profile.json",
        ])
        .unwrap();
        assert!(profiled.validate_intern_contract(false, false).is_ok());
        let diagnostic = Options::try_parse_from([
            "compare_cpython_corpus",
            "--diagnostic-assume-startup-intern-state",
        ])
        .unwrap();
        assert!(diagnostic.validate_intern_contract(false, false).is_ok());
        assert!(diagnostic.validate_intern_contract(true, false).is_err());
        assert!(diagnostic.validate_intern_contract(false, true).is_err());
        for gate in ["--require-all", "--require-rejection-parity"] {
            let options = Options::try_parse_from([
                "compare_cpython_corpus",
                "--diagnostic-assume-startup-intern-state",
                gate,
            ])
            .unwrap();
            assert!(options.validate_intern_contract(false, false).is_err());
        }
        assert!(
            Options::try_parse_from([
                "compare_cpython_corpus",
                "--diagnostic-assume-startup-intern-state",
                "--initial-intern-state",
                "profile.json",
            ])
            .is_err()
        );
    }

    #[test]
    fn unresolved_caller_identity_is_never_an_exact_match_or_rejection() {
        for oracle in [
            OracleOutcome::Compiled(vec![]),
            OracleOutcome::Rejected("SyntaxError".into()),
        ] {
            let compiled = capture_compiler(|| {
                Err(serc::CompileError::UnresolvedInternState {
                    value: "full".into(),
                })
            });
            let outcome = classify(Path::new("module.py"), oracle, compiled);
            assert_eq!(outcome.kind, OutcomeKind::UnresolvedInternState);
            assert!(outcome.kind.is_failure());
            assert!(outcome.detail.contains("full"));
        }
    }

    #[test]
    fn expected_unknown_counts_cannot_turn_missing_context_into_success() {
        let counts = BTreeMap::from([(OutcomeKind::UnresolvedInternState, 1)]);
        assert!(!validate_outcome_counts(&counts, 0, 1, 0, &counts, None, false, false).is_empty());
    }

    #[test]
    fn selected_python_version_is_parsed_and_validated() {
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let options = Options::try_parse_from([
                "compare_cpython_corpus",
                "--python-version",
                &version.to_string(),
            ])
            .unwrap();
            assert_eq!(options.python_version, version);
        }
        let error = Options::try_parse_from(["compare_cpython_corpus", "--python-version", "3.11"])
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn rejects_non_bare_extensions() {
        for extension in ["", ".pyi", "nested/pyi"] {
            let error =
                Options::try_parse_from(["compare_cpython_corpus", "--extension", extension])
                    .unwrap_err();
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
            let options =
                Options::try_parse_from(std::iter::once("compare_cpython_corpus").chain(args))
                    .unwrap();
            assert!(
                options
                    .expected_outcome_counts()
                    .unwrap_err()
                    .contains("may only be specified once")
            );
        }
    }

    #[test]
    fn explicit_profile_and_frozen_counts_are_preserved() {
        let options = Options::try_parse_from([
            "compare_cpython_corpus",
            "--python",
            "/cpython-3.14.5",
            "--source-bytes",
            "--initial-intern-state",
            "profile.json",
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
        assert_eq!(options.python, "/cpython-3.14.5");
        assert!(options.source_bytes && options.require_all && options.require_rejection_parity);
        assert_eq!(
            options.initial_intern_state.as_deref(),
            Some(Path::new("profile.json"))
        );
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
            let mut identity = PythonIdentity {
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
    fn frozen_native_oracle_script_remains_unchanged() {
        use sha2::{Digest, Sha256};

        assert_eq!(
            format!("{:x}", Sha256::digest(ORACLE.as_bytes())),
            "6812e016e7e871700001ad9f8f832c5998d553bc33c84afb1745dbd664a965fb"
        );
    }

    #[test]
    fn selected_target_reaches_both_input_modes() {
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
            for input in [
                CompilerInput::Text("value = 1\n".to_string()),
                CompilerInput::Bytes(b"value = 1\n".to_vec()),
            ] {
                let module = compile_module_input(&input, "target.py", &options, None).unwrap();
                assert_eq!(module.python_version(), python_version);
                assert_eq!(module.target(), python_version.target());
            }
        }
    }

    #[test]
    fn text_file_input_consumes_exactly_one_byte_order_mark() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bom.py");
        let filename = path.to_str().unwrap();
        let options = CompileOptions::default();
        for (source, expected) in [
            ("value = '\u{feff}'\n", Some("value = '\u{feff}'\n")),
            ("\u{feff}value = 1\n", Some("value = 1\n")),
            ("\u{feff}\u{feff}value = 1\n", None),
        ] {
            fs::write(&path, source).unwrap();
            let decoded = super::read_utf8_file(&path).unwrap();
            let text =
                compile_module_input(&CompilerInput::Text(decoded), filename, &options, None);
            let bytes = compile_module_input(
                &CompilerInput::Bytes(fs::read(&path).unwrap()),
                filename,
                &options,
                None,
            );
            if let Some(expected) = expected {
                let expected = serc::compile(expected, filename).unwrap().marshal();
                assert_eq!(text.unwrap().marshal(), expected);
                assert_eq!(bytes.unwrap().marshal(), expected);
            } else {
                assert!(text.is_err());
                assert!(bytes.is_err());
            }
        }
        assert!(
            compile_module_input(
                &CompilerInput::Text("\u{feff}value = 1\n".to_string()),
                filename,
                &options,
                None,
            )
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn mismatch_dumps_preserve_the_original_comparison_for_both_input_modes() {
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
        for source_bytes in [false, true] {
            fs::write(&path, source).unwrap();
            fs::write(&second_path, source).unwrap();
            fs::write(&oracle, "#!/bin/sh\nprintf 'oracle bytes'\n").unwrap();
            fs::set_permissions(&oracle, fs::Permissions::from_mode(0o700)).unwrap();
            let options = CompileOptions {
                initial_intern_state: state.clone(),
                ..CompileOptions::default()
            };
            let without_dump = super::compare(
                &path,
                oracle.to_str().unwrap(),
                source_bytes,
                &options,
                None,
                None,
                false,
            );
            assert_eq!(without_dump.kind, OutcomeKind::ByteMismatch);
            assert!(without_dump.mismatch_bytes.is_none());
            let mut outcome = super::compare(
                &path,
                oracle.to_str().unwrap(),
                source_bytes,
                &options,
                None,
                None,
                true,
            );
            assert_eq!(outcome.kind, OutcomeKind::ByteMismatch);
            fs::write(&oracle, "#!/bin/sh\nprintf 'other oracle bytes'\n").unwrap();
            let mut second_outcome = super::compare(
                &second_path,
                oracle.to_str().unwrap(),
                source_bytes,
                &options,
                None,
                None,
                true,
            );
            assert_eq!(second_outcome.kind, OutcomeKind::ByteMismatch);

            let dump = directory
                .path()
                .join(if source_bytes { "bytes" } else { "text" });
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
    fn explicit_state_reaches_both_input_modes_and_default_stays_unchanged() {
        let source = "f = lambda x: x in {'profile_specific_identifier', 'z'}\nx in {'profile_specific_identifier', 'z'}\n";
        let state = serc::InitialInternState::new(["profile_specific_identifier"]);
        for input in [
            CompilerInput::Text(source.to_string()),
            CompilerInput::Bytes(source.as_bytes().to_vec()),
        ] {
            let default =
                compile_module_input(&input, "profile.py", &CompileOptions::default(), None)
                    .unwrap()
                    .marshal();
            let supplied = compile_module_input(
                &input,
                "profile.py",
                &CompileOptions {
                    initial_intern_state: state.clone(),
                    ..CompileOptions::default()
                },
                None,
            )
            .unwrap()
            .marshal();
            assert_eq!(
                default,
                serc::compile(source, "profile.py").unwrap().marshal()
            );
            assert_eq!(
                supplied,
                serc::compile_with_intern_state(source, "profile.py", &state)
                    .unwrap()
                    .marshal()
            );
            assert_ne!(default, supplied);
        }
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
    fn oracle_distinguishes_compilation_rejections_and_io_failures() {
        let python = std::env::var("SERC_TEST_PYTHON").unwrap_or_else(|_| "python3.14".to_string());
        verify_python(&python, PythonVersion::Py314).unwrap();
        let directory = tempfile::tempdir().unwrap();

        for (filename, source, expected_status) in [
            ("valid.py", Some("pass\n"), 0),
            ("syntax.py", Some("if:\n"), 2),
            ("semantic.py", Some("return\n"), 2),
            ("missing.py", None, 1),
        ] {
            let path = directory.path().join(filename);
            if let Some(source) = source {
                fs::write(&path, source).unwrap();
            }
            let output = Command::new(&python)
                .args(["-I", "-S", "-c", ORACLE])
                .arg(&path)
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(expected_status), "{output:?}");
            if expected_status == 0 {
                let expected = serc::compile(source.unwrap(), path.to_str().unwrap())
                    .unwrap()
                    .marshal();
                assert_eq!(output.stdout, expected);
                assert!(output.stderr.is_empty(), "{output:?}");
            } else {
                assert!(output.stdout.is_empty(), "{output:?}");
                assert!(!output.stderr.is_empty(), "{output:?}");
            }
        }
    }

    #[test]
    fn cpython_rejection_requires_serc_rejection_for_text_and_bytes() {
        for source in [
            CompilerInput::Text("if:\n".to_string()),
            CompilerInput::Bytes(b"if:\n".to_vec()),
        ] {
            let compiled = compile_input(&source, "invalid.py", &CompileOptions::default(), None);
            let outcome = classify(
                Path::new("invalid.py"),
                OracleOutcome::Rejected("SyntaxError: invalid syntax".to_string()),
                compiled,
            );

            assert_eq!(outcome.kind, OutcomeKind::MatchedRejection);
            assert!(outcome.oracle_rejected);
            assert!(outcome.detail.contains("Serc rejected"));
        }
    }

    #[test]
    fn cpython_semantic_rejection_requires_serc_rejection() {
        let compiled = compile_input(
            &CompilerInput::Text("return\n".to_string()),
            "invalid.py",
            &CompileOptions::default(),
            None,
        );
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
    fn cpython_rejection_detects_serc_acceptance_for_text_and_bytes() {
        for source in [
            CompilerInput::Text("pass\n".to_string()),
            CompilerInput::Bytes(b"pass\n".to_vec()),
        ] {
            let compiled = compile_input(&source, "valid.py", &CompileOptions::default(), None);
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
        let source = CompilerInput::Text("def f():\n    '\\ud800'\n".to_string());
        let filename = "surrogate_docstring.py";
        let compiled = compile_input(&source, filename, &CompileOptions::default(), None);
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

        let options = Options::try_parse_from(["compare_cpython_corpus"]).unwrap();
        let default = discover(&roots, &options.extensions).unwrap();
        assert_eq!(default, vec![directory.path().join("module.py")]);

        let options =
            Options::try_parse_from(["compare_cpython_corpus", "--extension", "pyi"]).unwrap();
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
