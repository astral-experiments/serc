#![allow(clippy::print_stderr, clippy::print_stdout)]

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
    CompilerPanic,
    CpythonRejected,
    NonUtf8,
    OracleFailure,
}

impl OutcomeKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::ByteMismatch => "byte mismatch",
            Self::Unsupported => "unsupported",
            Self::ParseMismatch => "parse/decode mismatch",
            Self::CompilerPanic => "compiler panic",
            Self::CpythonRejected => "CPython rejected",
            Self::NonUtf8 => "non-UTF-8",
            Self::OracleFailure => "oracle failure",
        }
    }

    const fn is_failure(self) -> bool {
        !matches!(self, Self::Exact | Self::CpythonRejected)
    }
}

#[derive(Debug)]
struct Outcome {
    path: PathBuf,
    kind: OutcomeKind,
    detail: String,
}

enum CompilerInput {
    Bytes(Vec<u8>),
    Text(String),
}

#[derive(Debug)]
struct Options {
    python: String,
    roots: Vec<PathBuf>,
    extensions: Vec<String>,
    limit: Option<usize>,
    expected_files: Option<usize>,
    expected_exact: Option<usize>,
    expected_cpython_rejected: Option<usize>,
    require_all: bool,
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

    println!(
        "Compared {} Python files with {} in {:.2?}",
        outcomes.len(),
        options.python,
        started.elapsed()
    );
    for kind in [
        OutcomeKind::Exact,
        OutcomeKind::ByteMismatch,
        OutcomeKind::Unsupported,
        OutcomeKind::ParseMismatch,
        OutcomeKind::CompilerPanic,
        OutcomeKind::CpythonRejected,
        OutcomeKind::NonUtf8,
        OutcomeKind::OracleFailure,
    ] {
        println!("{:>20}: {}", kind.label(), counts.get(&kind).unwrap_or(&0));
    }

    for kind in [
        OutcomeKind::ByteMismatch,
        OutcomeKind::Unsupported,
        OutcomeKind::ParseMismatch,
        OutcomeKind::CompilerPanic,
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
        options.expected_exact,
        options.expected_cpython_rejected,
        options.require_all,
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
    expected_exact: Option<usize>,
    expected_cpython_rejected: Option<usize>,
    require_all: bool,
) -> Vec<String> {
    let count = |kind| counts.get(&kind).copied().unwrap_or_default();
    let exact = count(OutcomeKind::Exact);
    let cpython_rejected = count(OutcomeKind::CpythonRejected);
    let mut failures = Vec::new();

    if let Some(expected) = expected_exact
        && exact != expected
    {
        failures.push(format!(
            "found {exact} exact matches, but expected {expected}"
        ));
    }
    if let Some(expected) = expected_cpython_rejected
        && cpython_rejected != expected
    {
        failures.push(format!(
            "CPython rejected {cpython_rejected} files, but expected {expected}"
        ));
    }
    if require_all {
        let failed = counts
            .iter()
            .filter(|(kind, _)| kind.is_failure())
            .map(|(_, count)| count)
            .sum::<usize>();
        if failed != 0 {
            failures.push(format!("{failed} files did not match CPython exactly"));
        }
        if exact == 0 {
            failures.push("no files matched CPython exactly".to_string());
        }
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
    let mut expected_exact = None;
    let mut expected_cpython_rejected = None;
    let mut require_all = false;
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
                expected_exact = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "--expected-exact requires a number".to_string())?
                        .parse()
                        .map_err(|_| "--expected-exact requires a number".to_string())?,
                );
            }
            "--expected-cpython-rejected" => {
                expected_cpython_rejected = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "--expected-cpython-rejected requires a number".to_string())?
                        .parse()
                        .map_err(|_| "--expected-cpython-rejected requires a number".to_string())?,
                );
            }
            "--dump-mismatches" => {
                dump_mismatches =
                    Some(PathBuf::from(arguments.next().ok_or_else(|| {
                        "--dump-mismatches requires a directory".to_string()
                    })?));
            }
            "--require-all" => require_all = true,
            "--source-bytes" => source_bytes = true,
            "--help" | "-h" => {
                return Err(
                    "usage: compare_cpython_corpus [--python PATH] [--extension EXT] [--source-bytes] [--limit N] [--expected-files N] [--expected-exact N] [--expected-cpython-rejected N] [--examples N] [--require-all] [--dump-mismatches DIR] [ROOT ...]"
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
        expected_exact,
        expected_cpython_rejected,
        require_all,
        examples,
        dump_mismatches,
        source_bytes,
    })
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
        Ok(output) if output.status.success() => output.stdout,
        Ok(output) if output.status.code() == Some(2) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::CpythonRejected,
                detail: one_line(&output.stderr),
            };
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
            };
        }
        Err(error) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::OracleFailure,
                detail: error.to_string(),
            };
        }
    };

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
            };
        }
        Err(error) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::OracleFailure,
                detail: error.to_string(),
            };
        }
    };

    let filename = path.to_string_lossy();
    let compiled = std::panic::catch_unwind(|| match &source {
        CompilerInput::Bytes(source) => compile_bytes(source, &filename),
        CompilerInput::Text(source) => compile(source, &filename),
    });
    let ours = match compiled {
        Ok(Ok(module)) => module.marshal(),
        Ok(Err(CompileError::Unsupported(message))) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::Unsupported,
                detail: message,
            };
        }
        Ok(Err(CompileError::Parse(error))) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::ParseMismatch,
                detail: error.to_string(),
            };
        }
        Ok(Err(CompileError::InvalidPattern { message, .. })) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::ParseMismatch,
                detail: message,
            };
        }
        Ok(Err(CompileError::Decode(error))) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::ParseMismatch,
                detail: error.to_string(),
            };
        }
        Ok(Err(CompileError::Internal(message))) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::CompilerPanic,
                detail: message,
            };
        }
        Ok(Err(error)) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::CompilerPanic,
                detail: error.to_string(),
            };
        }
        Err(_) => {
            return Outcome {
                path: path.to_path_buf(),
                kind: OutcomeKind::CompilerPanic,
                detail: "panic while compiling".to_string(),
            };
        }
    };

    if ours == oracle {
        Outcome {
            path: path.to_path_buf(),
            kind: OutcomeKind::Exact,
            detail: format!("{} bytes", ours.len()),
        }
    } else {
        let first_difference = ours
            .iter()
            .zip(&oracle)
            .position(|(ours, oracle)| ours != oracle)
            .unwrap_or_else(|| ours.len().min(oracle.len()));
        Outcome {
            path: path.to_path_buf(),
            kind: OutcomeKind::ByteMismatch,
            detail: {
                let ours_byte = ours.get(first_difference).copied();
                let oracle_byte = oracle.get(first_difference).copied();
                format!(
                    "first difference at byte {first_difference} ({ours_byte:?} != {oracle_byte:?}); ours={} bytes, CPython={} bytes",
                    ours.len(),
                    oracle.len()
                )
            },
        }
    }
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
    use super::{ORACLE, OutcomeKind, discover, validate_outcome_counts};
    use std::collections::BTreeMap;
    use std::fs;

    #[test]
    fn require_all_rejects_a_corpus_without_exact_matches() {
        let counts = BTreeMap::from([(OutcomeKind::CpythonRejected, 3)]);

        let failures = validate_outcome_counts(&counts, None, None, true);

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
        let counts = BTreeMap::from([(OutcomeKind::Exact, 2), (OutcomeKind::CpythonRejected, 1)]);

        assert!(validate_outcome_counts(&counts, Some(2), Some(1), true).is_empty());
        assert_eq!(
            validate_outcome_counts(&counts, Some(3), Some(2), false),
            [
                "found 2 exact matches, but expected 3",
                "CPython rejected 1 files, but expected 2",
            ]
        );
    }
}
