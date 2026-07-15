#![allow(clippy::print_stderr, clippy::print_stdout)]

use std::any::Any;
use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

use serc::{CPYTHON_TARGET, CompileError, compile, compile_bytes};
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
    MatchedRejection,
    RejectionMismatch,
    NonUtf8,
    OracleFailure,
}

impl OutcomeKind {
    const ALL: [Self; 9] = [
        Self::Exact,
        Self::ByteMismatch,
        Self::Unsupported,
        Self::ParseMismatch,
        Self::CompilerFailure,
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
    Rejected(String),
    Failed(String),
}

#[derive(Debug)]
struct Options {
    python: String,
    roots: Vec<PathBuf>,
    extensions: Vec<String>,
    limit: Option<usize>,
    expected_files: Option<usize>,
    expected_outcomes: BTreeMap<OutcomeKind, usize>,
    expected_cpython_rejected: Option<usize>,
    require_all: bool,
    require_rejection_parity: bool,
    examples: usize,
    dump_mismatches: Option<PathBuf>,
    source_bytes: bool,
}

fn main() -> ExitCode {
    let options = match parse_options() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    if let Err(message) = verify_python(&options.python) {
        eprintln!("{message}");
        return ExitCode::from(2);
    }
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

    let started = Instant::now();
    let outcomes: Vec<_> = pool.install(|| {
        paths
            .par_iter()
            .map(|path| compare(path, &options.python, options.source_bytes))
            .collect()
    });

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
        if let Err(error) =
            dump_mismatches(&outcomes, &options.python, directory, options.source_bytes)
        {
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
        &options.expected_outcomes,
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

fn verify_python(python: &str) -> Result<(), String> {
    let version = CPYTHON_TARGET.version_string();
    let magic = CPYTHON_TARGET.magic_hex();
    let output = Command::new(python)
        .args([
            "-I",
            "-S",
            "-c",
            "import importlib.util, sys; actual = sys.version_info[:3]; magic = importlib.util.MAGIC_NUMBER.hex(); print(f'{sys.implementation.name} {'.'.join(map(str, actual))} magic {magic}'); expected = tuple(map(int, sys.argv[2].split('.'))); raise SystemExit(sys.implementation.name != sys.argv[1] or actual != expected or magic != sys.argv[3])",
            CPYTHON_TARGET.implementation,
            version.as_str(),
            magic.as_str(),
        ])
        .output()
        .map_err(|error| format!("failed to run {python}: {error}"))?;
    let identity = one_line(&output.stdout);
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{python} is {identity}; the compiler and oracle require {} {} with magic {}",
            CPYTHON_TARGET.implementation, version, magic
        ))
    }
}

fn parse_options() -> Result<Options, String> {
    let mut python = "python3".to_string();
    let mut roots = Vec::new();
    let mut extensions = vec!["py".to_string()];
    let mut limit = None;
    let mut expected_files = None;
    let mut expected_outcomes = BTreeMap::new();
    let mut expected_cpython_rejected = None;
    let mut require_all = false;
    let mut require_rejection_parity = false;
    let mut examples = 10;
    let mut dump_mismatches = None;
    let mut source_bytes = false;
    let mut arguments = env::args().skip(1);

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--python" => {
                python = arguments
                    .next()
                    .ok_or_else(|| "--python requires an executable".to_string())?;
            }
            "--limit" => {
                limit = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "--limit requires a number".to_string())?
                        .parse()
                        .map_err(|_| "--limit requires a number".to_string())?,
                );
            }
            "--examples" => {
                examples = arguments
                    .next()
                    .ok_or_else(|| "--examples requires a number".to_string())?
                    .parse()
                    .map_err(|_| "--examples requires a number".to_string())?;
            }
            "--extension" => {
                let extension = arguments
                    .next()
                    .ok_or_else(|| "--extension requires a value".to_string())?;
                if extension.is_empty()
                    || extension.starts_with('.')
                    || extension.contains(std::path::is_separator)
                {
                    return Err(
                        "--extension requires a non-empty bare extension such as `pyi`".to_string(),
                    );
                }
                if !extensions.contains(&extension) {
                    extensions.push(extension);
                }
            }
            "--expected-files" => {
                expected_files = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "--expected-files requires a number".to_string())?
                        .parse()
                        .map_err(|_| "--expected-files requires a number".to_string())?,
                );
            }
            "--expected-exact" => {
                insert_expected_outcome(
                    &mut expected_outcomes,
                    OutcomeKind::Exact,
                    parse_count(&mut arguments, "--expected-exact")?,
                    "--expected-exact",
                )?;
            }
            "--expected-cpython-rejected" => {
                let count = parse_count(&mut arguments, "--expected-cpython-rejected")?;
                if expected_cpython_rejected.replace(count).is_some() {
                    return Err(
                        "--expected-cpython-rejected may only be specified once".to_string()
                    );
                }
            }
            "--expected-outcome" => {
                let expected = arguments
                    .next()
                    .ok_or_else(|| "--expected-outcome requires a KIND=COUNT value".to_string())?;
                let (kind, count) = parse_expected_outcome(&expected)?;
                insert_expected_outcome(&mut expected_outcomes, kind, count, "--expected-outcome")?;
            }
            "--dump-mismatches" => {
                dump_mismatches =
                    Some(PathBuf::from(arguments.next().ok_or_else(|| {
                        "--dump-mismatches requires a directory".to_string()
                    })?));
            }
            "--require-all" => require_all = true,
            "--require-rejection-parity" => require_rejection_parity = true,
            "--source-bytes" => source_bytes = true,
            "--help" | "-h" => {
                return Err(
                    "usage: compare_cpython_corpus [--python PATH] [--extension EXT] [--source-bytes] [--limit N] [--expected-files N] [--expected-exact N] [--expected-cpython-rejected N] [--examples N] [--require-all] [--require-rejection-parity] [--dump-mismatches DIR] [ROOT ...]"
                        .to_string(),
                );
            }
            _ if argument.starts_with('-') => {
                return Err(format!("unknown option: {argument}"));
            }
            _ => roots.push(PathBuf::from(argument)),
        }
    }

    if roots.is_empty() {
        let ruff = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/ruff");
        roots = [
            "crates/ty_python_semantic/resources/corpus",
            "crates/ruff_python_parser/resources/valid",
            "crates/ruff_python_parser/resources/inline/ok",
            "crates/ruff_linter/resources/test/fixtures",
            "crates/ruff_python_formatter/resources/test/fixtures",
        ]
        .into_iter()
        .map(|root| ruff.join(root))
        .collect();
    }

    Ok(Options {
        python,
        roots,
        extensions,
        limit,
        expected_files,
        expected_outcomes,
        expected_cpython_rejected,
        require_all,
        require_rejection_parity,
        examples,
        dump_mismatches,
        source_bytes,
    })
}

fn parse_count(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<usize, String> {
    arguments
        .next()
        .ok_or_else(|| format!("{option} requires a number"))?
        .parse()
        .map_err(|_| format!("{option} requires a number"))
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

fn insert_expected_outcome(
    outcomes: &mut BTreeMap<OutcomeKind, usize>,
    kind: OutcomeKind,
    count: usize,
    option: &str,
) -> Result<(), String> {
    if outcomes.insert(kind, count).is_some() {
        return Err(format!("{option} may only be specified once"));
    }
    Ok(())
}

fn dump_mismatches(
    outcomes: &[Outcome],
    python: &str,
    directory: &Path,
    source_bytes: bool,
) -> Result<(), std::io::Error> {
    fs_err::create_dir_all(directory)?;
    outcomes
        .iter()
        .filter(|outcome| outcome.kind == OutcomeKind::ByteMismatch)
        .enumerate()
        .try_for_each(|(index, outcome)| {
            let source = if source_bytes {
                CompilerInput::Bytes(fs_err::read(&outcome.path)?)
            } else {
                CompilerInput::Text(fs_err::read_to_string(&outcome.path)?)
            };
            let filename = outcome.path.to_string_lossy();
            let ours = match &source {
                CompilerInput::Bytes(source) => compile_bytes(source, &filename),
                CompilerInput::Text(source) => compile(source, &filename),
            }
            .expect("a mismatch compiled successfully in the first pass")
            .marshal();
            let oracle = Command::new(python)
                .args(["-I", "-S", "-c", ORACLE])
                .arg(&outcome.path)
                .output()?
                .stdout;
            let stem = format!("{index:04}");
            fs_err::write(directory.join(format!("{stem}.path")), filename.as_bytes())?;
            fs_err::write(directory.join(format!("{stem}.ours.marshal")), ours)?;
            fs_err::write(directory.join(format!("{stem}.cpython.marshal")), oracle)
        })
}

fn discover(roots: &[PathBuf], extensions: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for root in roots {
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = entry.map_err(|error| {
                format!("failed to walk corpus root {}: {error}", root.display())
            })?;
            if entry.file_type().is_file()
                && entry.path().extension().is_some_and(|extension| {
                    extensions
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

fn compare(path: &Path, python: &str, source_bytes: bool) -> Outcome {
    let oracle = match Command::new(python)
        .args(["-I", "-S", "-c", ORACLE])
        .arg(path)
        .output()
    {
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
            };
        }
        Err(error) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::OracleFailure,
                detail: error.to_string(),
                oracle_rejected: false,
            };
        }
    };
    let oracle_rejected = matches!(&oracle, OracleOutcome::Rejected(_));

    let source = if source_bytes {
        fs_err::read(path).map(CompilerInput::Bytes)
    } else {
        fs_err::read_to_string(path).map(CompilerInput::Text)
    };
    let source = match source {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::NonUtf8,
                detail: error.to_string(),
                oracle_rejected,
            };
        }
        Err(error) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::OracleFailure,
                detail: error.to_string(),
                oracle_rejected,
            };
        }
    };

    let filename = path.to_string_lossy();
    let compiled = compile_input(&source, &filename);

    classify(path, oracle, compiled)
}

fn compile_input(source: &CompilerInput, filename: &str) -> CompilerOutcome {
    capture_compiler(|| {
        match source {
            CompilerInput::Bytes(source) => compile_bytes(source, filename),
            CompilerInput::Text(source) => compile(source, filename),
        }
        .map(|module| module.marshal())
    })
}

fn capture_compiler(compile: impl FnOnce() -> Result<Vec<u8>, CompileError>) -> CompilerOutcome {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(compile)) {
        Ok(Ok(marshal)) => CompilerOutcome::Compiled(marshal),
        Ok(Err(CompileError::Unsupported(message))) => CompilerOutcome::Unsupported(message),
        Ok(Err(CompileError::Parse(error))) => CompilerOutcome::Rejected(error.to_string()),
        Ok(Err(
            CompileError::InvalidPattern { message, .. }
            | CompileError::InvalidSyntax { message, .. },
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

fn classify(path: &Path, oracle: OracleOutcome, compiled: CompilerOutcome) -> Outcome {
    let oracle_rejected = matches!(&oracle, OracleOutcome::Rejected(_));
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
                (
                    OutcomeKind::ByteMismatch,
                    format!(
                        "first difference at byte {first_difference} ({ours_byte:?} != {oracle_byte:?}); ours={} bytes, CPython={} bytes",
                        ours.len(),
                        oracle.len()
                    ),
                )
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
        CompilerInput, CompilerOutcome, ORACLE, OracleOutcome, OutcomeKind, capture_compiler,
        classify, compile_input, discover, validate_outcome_counts,
    };
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

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
    fn oracle_classifies_only_compile_exceptions_as_rejected() {
        let read = ORACLE.find("source = path.read_bytes()").unwrap();
        let try_compile = ORACLE.find("try:\n    code = compile(source").unwrap();
        let compile_exception = ORACLE.find("except Exception as error:").unwrap();
        let marshal = ORACLE
            .find("sys.stdout.buffer.write(marshal.dumps(code))")
            .unwrap();

        assert!(read < try_compile);
        assert!(try_compile < compile_exception);
        assert!(compile_exception < marshal);
    }

    #[test]
    fn cpython_rejection_requires_serc_rejection_for_text_and_bytes() {
        for source in [
            CompilerInput::Text("if:\n".to_string()),
            CompilerInput::Bytes(b"if:\n".to_vec()),
        ] {
            let compiled = compile_input(&source, "invalid.py");
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
        let compiled = compile_input(&CompilerInput::Text("return\n".to_string()), "invalid.py");
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
            let compiled = compile_input(&source, "valid.py");
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

        let default = discover(&roots, &["py".to_string()]).unwrap();
        assert_eq!(default, vec![directory.path().join("module.py")]);

        let with_stubs = discover(&roots, &["py".to_string(), "pyi".to_string()]).unwrap();
        assert_eq!(
            with_stubs,
            vec![
                directory.path().join("module.py"),
                directory.path().join("module.pyi")
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
