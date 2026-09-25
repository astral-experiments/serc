#![allow(clippy::print_stderr, clippy::print_stdout)]

use std::env;
use std::fmt::Write as _;
use std::hint::black_box;
use std::io::{self, BufRead, BufReader, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use cpu_time::ProcessTime;
use serc::{CPYTHON_TARGET, CompiledModule, compile};
use walkdir::WalkDir;

const BOOTSTRAP_SAMPLES: usize = 10_000;
const BOOTSTRAP_SEED: u64 = 0x75_76_2d_62_79_74_65;

const PYTHON_HELPER: &str = r#"
import importlib.util
import marshal
import sys
import time
import warnings

EXPECTED_IMPLEMENTATION = sys.argv[1]
EXPECTED_VERSION = tuple(map(int, sys.argv[2].split(".")))
EXPECTED_MAGIC = sys.argv[3]

implementation = sys.implementation.name
version = sys.version_info[:3]
magic = importlib.util.MAGIC_NUMBER.hex()
if (
    implementation != EXPECTED_IMPLEMENTATION
    or version != EXPECTED_VERSION
    or sys.version_info[3:] != ("final", 0)
    or magic != EXPECTED_MAGIC
):
    print(
        f"ERROR\texpected {EXPECTED_IMPLEMENTATION} "
        f"{'.'.join(map(str, EXPECTED_VERSION))} "
        f"magic {EXPECTED_MAGIC}, "
        f"got {implementation} {'.'.join(map(str, version))} "
        f"{sys.version_info.releaselevel} {sys.version_info.serial} magic {magic}",
        flush=True,
    )
    raise SystemExit(2)

print(
    f"READY\t{implementation}\t{'.'.join(map(str, version))}\t{magic}",
    flush=True,
)
warnings.simplefilter("ignore", SyntaxWarning)
stream = sys.stdin.buffer
sources = []
precompiled = []

def read_exact(length):
    value = stream.read(length)
    if len(value) != length:
        raise EOFError(f"expected {length} bytes, got {len(value)}")
    return value

while True:
    command = stream.readline()
    if not command:
        raise SystemExit(0)
    fields = command.decode("ascii").rstrip("\n").split("\t")
    if fields[0] == "INIT":
        count = int(fields[1])
        accepted = []
        rejected = 0
        accepted_bytes = 0
        for index in range(count):
            header = stream.readline().decode("ascii").rstrip("\n").split("\t")
            if len(header) != 3 or header[0] != "FILE":
                raise RuntimeError(f"invalid FILE header: {header!r}")
            filename = read_exact(int(header[1])).decode("utf-8")
            source = read_exact(int(header[2])).decode("utf-8")
            try:
                code = compile(
                    source,
                    filename,
                    "exec",
                    dont_inherit=True,
                    optimize=0,
                )
            except Exception:
                rejected += 1
                print(f"REJECT\t{index}", flush=True)
                continue
            accepted.append((filename, source))
            precompiled.append(code)
            accepted_bytes += len(source.encode("utf-8"))
            serialized = marshal.dumps(code)
            print(f"ACCEPT\t{index}\t{len(serialized)}", flush=True)
            sys.stdout.buffer.write(serialized)
            sys.stdout.buffer.flush()
        sources = accepted
        print(
            f"MANIFEST\t{len(accepted)}\t{rejected}\t{accepted_bytes}",
            flush=True,
        )
    elif fields[0] == "RUN":
        phase = fields[1]
        if phase == "compiler_core":
            wall_started = time.perf_counter_ns()
            process_cpu_started = time.process_time_ns()
            outputs = [
                compile(
                    source,
                    filename,
                    "exec",
                    dont_inherit=True,
                    optimize=0,
                )
                for filename, source in sources
            ]
            process_cpu_ns = time.process_time_ns() - process_cpu_started
            wall_elapsed_ns = time.perf_counter_ns() - wall_started
            observed = sum(len(output.co_code) for output in outputs)
        elif phase == "compile_marshal":
            wall_started = time.perf_counter_ns()
            process_cpu_started = time.process_time_ns()
            outputs = [
                marshal.dumps(
                    compile(
                        source,
                        filename,
                        "exec",
                        dont_inherit=True,
                        optimize=0,
                    )
                )
                for filename, source in sources
            ]
            process_cpu_ns = time.process_time_ns() - process_cpu_started
            wall_elapsed_ns = time.perf_counter_ns() - wall_started
            observed = sum(len(output) for output in outputs)
        elif phase == "marshal_only":
            wall_started = time.perf_counter_ns()
            process_cpu_started = time.process_time_ns()
            outputs = [marshal.dumps(output) for output in precompiled]
            process_cpu_ns = time.process_time_ns() - process_cpu_started
            wall_elapsed_ns = time.perf_counter_ns() - wall_started
            observed = sum(len(output) for output in outputs)
        else:
            raise RuntimeError(f"unknown phase: {phase}")
        output_count = len(outputs)
        del outputs
        print(
            f"RESULT\t{wall_elapsed_ns}\t{process_cpu_ns}"
            f"\t{output_count}\t{observed}",
            flush=True,
        )
    elif fields[0] == "EXIT":
        raise SystemExit(0)
    else:
        raise RuntimeError(f"unknown command: {fields!r}")
"#;

#[derive(Clone, Debug, Eq, PartialEq, Parser)]
#[command(about = "Benchmark Serc against the pinned CPython target")]
struct Options {
    #[arg(long, value_name = "PATH")]
    python: String,
    #[arg(long, value_enum)]
    phase: Phase,
    #[arg(long, value_parser = parse_positive)]
    warmups: usize,
    #[arg(long, value_parser = parse_positive)]
    samples: usize,
    #[arg(long = "cooldown-ms", value_parser = parse_duration, value_name = "N")]
    cooldown: Duration,
    #[arg(long, value_name = "PATH")]
    output: PathBuf,
    #[arg(required = true, value_name = "ROOT")]
    roots: Vec<PathBuf>,
    #[arg(long, value_parser = parse_positive)]
    limit: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "snake_case")]
enum Phase {
    CompilerCore,
    CompileMarshal,
    MarshalOnly,
}

impl Phase {
    const fn label(self) -> &'static str {
        match self {
            Self::CompilerCore => "compiler_core",
            Self::CompileMarshal => "compile_marshal",
            Self::MarshalOnly => "marshal_only",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Engine {
    Rust,
    Cpython,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Metric {
    Wall,
    ProcessCpu,
}

impl Metric {
    const ALL: [Self; 2] = [Self::Wall, Self::ProcessCpu];

    const fn label(self) -> &'static str {
        match self {
            Self::Wall => "wall",
            Self::ProcessCpu => "process_cpu",
        }
    }

    const fn value(self, timings: &Timings) -> u64 {
        match self {
            Self::Wall => timings.wall_elapsed_ns,
            Self::ProcessCpu => timings.process_cpu_ns,
        }
    }
}

impl Engine {
    const fn label(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Cpython => "cpython",
        }
    }
}

#[derive(Debug)]
struct SourceFile {
    path: PathBuf,
    filename: String,
    source: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Timings {
    wall_elapsed_ns: u64,
    process_cpu_ns: u64,
}

impl Timings {
    fn new(wall_elapsed_ns: u64, process_cpu_ns: u64) -> Result<Self, String> {
        if wall_elapsed_ns == 0 {
            return Err("wall-clock sample duration is zero".to_string());
        }
        if process_cpu_ns == 0 {
            return Err("process CPU sample duration is zero".to_string());
        }
        Ok(Self {
            wall_elapsed_ns,
            process_cpu_ns,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SampleResult {
    timings: Timings,
    count: usize,
    observed: usize,
}

#[derive(Debug)]
struct PythonRunner {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(Debug)]
struct PythonManifest {
    accepted: Vec<AcceptedFile>,
    rejected: usize,
    source_bytes: usize,
}

impl PythonManifest {
    fn into_counts(self) -> ManifestCounts {
        ManifestCounts {
            accepted: self.accepted.len(),
            rejected: self.rejected,
            source_bytes: self.source_bytes,
        }
    }
}

#[derive(Debug)]
struct ManifestCounts {
    accepted: usize,
    rejected: usize,
    source_bytes: usize,
}

#[derive(Debug, Eq, PartialEq)]
struct AcceptedFile {
    source_index: usize,
    marshaled: Vec<u8>,
}

#[derive(Debug)]
struct ArtifactIdentity {
    profile: String,
    serc_commit: String,
    serc_dirty: bool,
    serc_diff_hash: String,
    cargo_lock_hash: String,
    corpus_content_hash: String,
    benchmark_executable: String,
    benchmark_executable_hash: String,
    python_realpath: String,
    python_executable_hash: String,
    hash_scheme: String,
}

#[derive(Clone, Copy, Debug)]
struct Summary {
    median: f64,
    p95: f64,
    mad: f64,
    relative_mad: f64,
    half_delta: f64,
}

#[derive(Clone, Copy, Debug)]
struct BlockSample {
    rust_mean_ns: f64,
    cpython_mean_ns: f64,
    ratio: f64,
}

#[derive(Clone, Copy, Debug)]
struct RatioStatistics {
    summary: Summary,
    bootstrap_low: f64,
    bootstrap_high: f64,
    abba_median: Option<f64>,
    baab_median: Option<f64>,
    order_delta: Option<f64>,
    stable: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let options = Options::parse();
    let paths = discover(&options.roots, options.limit)?;
    if paths.is_empty() {
        return Err("no Python files discovered".to_string());
    }
    let sources = load_sources(paths)?;
    let discovered_source_bytes = sources
        .iter()
        .map(|source| source.source.len())
        .sum::<usize>();
    let identity = ArtifactIdentity::capture(&options.python, &sources)?;

    let mut python = PythonRunner::start(&options.python)?;
    let manifest = python.initialize(&sources)?;
    if manifest.accepted.len() + manifest.rejected != sources.len() {
        return Err(format!(
            "CPython manifest is incomplete: {} accepted + {} rejected != {} discovered",
            manifest.accepted.len(),
            manifest.rejected,
            sources.len()
        ));
    }
    let accepted = manifest
        .accepted
        .iter()
        .map(|file| {
            sources.get(file.source_index).ok_or_else(|| {
                format!(
                    "CPython returned invalid accepted index {}",
                    file.source_index
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let accepted_source_bytes = accepted
        .iter()
        .map(|source| source.source.len())
        .sum::<usize>();
    if accepted_source_bytes != manifest.source_bytes {
        return Err(format!(
            "source-byte disagreement: Rust={accepted_source_bytes}, CPython={}",
            manifest.source_bytes
        ));
    }

    let precompiled = accepted
        .iter()
        .map(|source| {
            compile(&source.source, &source.filename).map_err(|error| {
                format!(
                    "Rust failed to compile CPython-accepted file {}: {error}",
                    source.path.display()
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    verify_exact_marshal(&accepted, &precompiled, &manifest.accepted)?;
    let manifest = manifest.into_counts();

    let mut blocks = Vec::with_capacity(expected_row_count(&options)? / 4);
    run_blocks(
        options.phase,
        options.warmups,
        options.cooldown,
        &accepted,
        &precompiled,
        &mut python,
        &mut blocks,
    )?;
    run_blocks(
        options.phase,
        options.samples,
        options.cooldown,
        &accepted,
        &precompiled,
        &mut python,
        &mut blocks,
    )?;
    python.finish()?;

    let metadata = metadata(
        &options,
        &identity,
        &sources,
        discovered_source_bytes,
        &manifest,
    )?;
    write_raw(&options, &metadata, &accepted, &blocks)?;
    print_summaries(&blocks[options.warmups..], options.phase)?;
    Ok(())
}

fn expected_row_count(options: &Options) -> Result<usize, String> {
    options
        .warmups
        .checked_add(options.samples)
        .and_then(|blocks| blocks.checked_mul(4))
        .ok_or_else(|| "warmup and sample counts produce too many observations".to_string())
}

fn parse_positive(value: &str) -> Result<usize, String> {
    let value = value
        .parse::<usize>()
        .map_err(|_| "expected a positive number".to_string())?;
    if value == 0 {
        return Err("expected a positive number".to_string());
    }
    Ok(value)
}

fn parse_duration(value: &str) -> Result<Duration, String> {
    value
        .parse::<u64>()
        .map(Duration::from_millis)
        .map_err(|_| "--cooldown-ms requires a non-negative integer".to_string())
}

fn discover(roots: &[PathBuf], limit: Option<usize>) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for root in roots {
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = entry.map_err(|error| {
                format!("failed to walk corpus root {}: {error}", root.display())
            })?;
            if entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "py")
            {
                paths.push(entry.into_path());
            }
        }
    }
    paths.sort();
    paths.dedup();
    if let Some(limit) = limit {
        paths.truncate(limit);
    }
    Ok(paths)
}

fn load_sources(paths: Vec<PathBuf>) -> Result<Vec<SourceFile>, String> {
    paths
        .into_iter()
        .map(|path| {
            let bytes = fs_err::read(&path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
            let source = std::str::from_utf8(&bytes)
                .map_err(|error| format!("{} is not UTF-8: {error}", path.display()))?;
            let normalized;
            let source = if source.contains('\r') {
                normalized = source.replace("\r\n", "\n").replace('\r', "\n");
                normalized.as_str()
            } else {
                source
            };
            let source = source
                .strip_prefix('\u{feff}')
                .unwrap_or(source)
                .to_string();
            let absolute = path
                .canonicalize()
                .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()))?;
            let filename = absolute
                .to_str()
                .ok_or_else(|| format!("path is not UTF-8: {}", absolute.display()))?
                .to_string();
            Ok(SourceFile {
                path: absolute,
                filename,
                source,
            })
        })
        .collect()
}

impl PythonRunner {
    fn start(python: &str) -> Result<Self, String> {
        let version = CPYTHON_TARGET.version_string();
        let magic = CPYTHON_TARGET.magic_hex();
        let mut child = Command::new(python)
            .args([
                "-I",
                "-S",
                "-u",
                "-c",
                PYTHON_HELPER,
                CPYTHON_TARGET.implementation,
                version.as_str(),
                magic.as_str(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("failed to start {python}: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "failed to open CPython stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to open CPython stdout".to_string())?;
        let mut runner = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        };
        let ready = runner.read_line()?;
        let expected = format!(
            "READY\t{}\t{}\t{}",
            CPYTHON_TARGET.implementation, version, magic
        );
        if ready != expected {
            return Err(format!("invalid CPython identity: {ready}"));
        }
        Ok(runner)
    }

    fn initialize(&mut self, sources: &[SourceFile]) -> Result<PythonManifest, String> {
        writeln!(self.stdin, "INIT\t{}", sources.len())
            .map_err(|error| format!("failed to initialize CPython: {error}"))?;
        let mut accepted_files = Vec::new();
        for (index, source) in sources.iter().enumerate() {
            writeln!(
                self.stdin,
                "FILE\t{}\t{}",
                source.filename.len(),
                source.source.len()
            )
            .and_then(|()| self.stdin.write_all(source.filename.as_bytes()))
            .and_then(|()| self.stdin.write_all(source.source.as_bytes()))
            .and_then(|()| self.stdin.flush())
            .map_err(|error| format!("failed to send {}: {error}", source.path.display()))?;

            // Drain this file's response before sending the next source: both pipe
            // buffers can fill when CPython returns a large marshal payload.
            let line = self.read_line()?;
            let fields = line.split('\t').collect::<Vec<_>>();
            match fields.as_slice() {
                ["ACCEPT", accepted_index, length]
                    if accepted_index.parse::<usize>() == Ok(index) =>
                {
                    let length = length
                        .parse::<usize>()
                        .map_err(|_| format!("invalid marshal length: {line}"))?;
                    let mut bytes = vec![0; length];
                    self.stdout.read_exact(&mut bytes).map_err(|error| {
                        format!("failed to read CPython marshal output: {error}")
                    })?;
                    accepted_files.push(AcceptedFile {
                        source_index: index,
                        marshaled: bytes,
                    });
                }
                ["REJECT", rejected_index] if rejected_index.parse::<usize>() == Ok(index) => {}
                _ => {
                    return Err(format!(
                        "invalid CPython file response for index {index}: {line}"
                    ));
                }
            }
        }

        let line = self.read_line()?;
        let fields = line.split('\t').collect::<Vec<_>>();
        let ["MANIFEST", accepted, rejected, source_bytes] = fields.as_slice() else {
            return Err(format!("invalid CPython manifest response: {line}"));
        };
        let accepted = accepted
            .parse::<usize>()
            .map_err(|_| format!("invalid manifest: {line}"))?;
        if accepted != accepted_files.len() {
            return Err(format!(
                "CPython reported {accepted} accepted files but sent {} indices",
                accepted_files.len()
            ));
        }
        Ok(PythonManifest {
            accepted: accepted_files,
            rejected: rejected
                .parse::<usize>()
                .map_err(|_| format!("invalid manifest: {line}"))?,
            source_bytes: source_bytes
                .parse::<usize>()
                .map_err(|_| format!("invalid manifest: {line}"))?,
        })
    }

    fn run_phase(&mut self, phase: Phase) -> Result<SampleResult, String> {
        writeln!(self.stdin, "RUN\t{}", phase.label())
            .and_then(|()| self.stdin.flush())
            .map_err(|error| format!("failed to request CPython sample: {error}"))?;
        let line = self.read_line()?;
        parse_sample_response(&line)
    }

    fn read_line(&mut self) -> Result<String, String> {
        let mut line = String::new();
        let bytes = self
            .stdout
            .read_line(&mut line)
            .map_err(|error| format!("failed to read CPython response: {error}"))?;
        if bytes == 0 {
            return Err("CPython helper closed its output unexpectedly".to_string());
        }
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }

    fn finish(mut self) -> Result<(), String> {
        writeln!(self.stdin, "EXIT")
            .and_then(|()| self.stdin.flush())
            .map_err(|error| format!("failed to stop CPython helper: {error}"))?;
        drop(self.stdin);
        let status = self
            .child
            .wait()
            .map_err(|error| format!("failed to wait for CPython helper: {error}"))?;
        if !status.success() {
            return Err(format!("CPython helper exited with {status}"));
        }
        Ok(())
    }
}

fn verify_exact_marshal(
    accepted: &[&SourceFile],
    precompiled: &[CompiledModule],
    cpython_accepted: &[AcceptedFile],
) -> Result<(), String> {
    if accepted.is_empty() {
        return Err("exact-marshal preflight has no CPython-accepted files".to_string());
    }
    if precompiled.len() != accepted.len() || cpython_accepted.len() != accepted.len() {
        return Err(format!(
            "exact-marshal preflight is incomplete: {} sources, {} Rust modules, {} CPython outputs",
            accepted.len(),
            precompiled.len(),
            cpython_accepted.len(),
        ));
    }
    for ((source, module), file) in accepted.iter().zip(precompiled).zip(cpython_accepted) {
        let cpython = &file.marshaled;
        let rust = module.marshal();
        if rust != *cpython {
            let difference = rust
                .iter()
                .zip(cpython)
                .position(|(rust, cpython)| rust != cpython)
                .unwrap_or_else(|| rust.len().min(cpython.len()));
            return Err(format!(
                "exact-marshal preflight failed for {} at byte {difference}: Rust={} bytes, CPython={} bytes",
                source.path.display(),
                rust.len(),
                cpython.len(),
            ));
        }
    }
    Ok(())
}

fn parse_sample_response(line: &str) -> Result<SampleResult, String> {
    let fields = line.split('\t').collect::<Vec<_>>();
    let ["RESULT", wall_elapsed_ns, process_cpu_ns, count, observed] = fields.as_slice() else {
        return Err(format!("invalid CPython sample response: {line}"));
    };
    Ok(SampleResult {
        timings: Timings::new(
            wall_elapsed_ns
                .parse::<u64>()
                .map_err(|_| format!("invalid CPython wall elapsed time: {line}"))?,
            process_cpu_ns
                .parse::<u64>()
                .map_err(|_| format!("invalid CPython process CPU time: {line}"))?,
        )?,
        count: count
            .parse::<usize>()
            .map_err(|_| format!("invalid CPython output count: {line}"))?,
        observed: observed
            .parse::<usize>()
            .map_err(|_| format!("invalid CPython observed bytes: {line}"))?,
    })
}

fn run_blocks(
    phase: Phase,
    count: usize,
    cooldown: Duration,
    accepted: &[&SourceFile],
    precompiled: &[CompiledModule],
    python: &mut PythonRunner,
    blocks: &mut Vec<[Timings; 4]>,
) -> Result<(), String> {
    let mut sample = |engine| {
        thread::sleep(cooldown);
        let timings = match engine {
            Engine::Rust => run_rust_phase(phase, accepted, precompiled)?,
            Engine::Cpython => {
                let sample = python.run_phase(phase)?;
                if sample.count != accepted.len() {
                    return Err(format!(
                        "CPython returned {} outputs for {} accepted files",
                        sample.count,
                        accepted.len()
                    ));
                }
                black_box(sample.observed);
                sample.timings
            }
        };
        Ok(timings)
    };
    for block in 1..=count {
        let [first, second, third, fourth] = block_order(block);
        blocks.push([
            sample(first)?,
            sample(second)?,
            sample(third)?,
            sample(fourth)?,
        ]);
    }
    Ok(())
}

fn block_order(block: usize) -> [Engine; 4] {
    if block % 2 == 1 {
        [Engine::Rust, Engine::Cpython, Engine::Cpython, Engine::Rust]
    } else {
        [Engine::Cpython, Engine::Rust, Engine::Rust, Engine::Cpython]
    }
}

fn block_order_label(block: usize) -> &'static str {
    if block % 2 == 1 { "ABBA" } else { "BAAB" }
}

fn run_rust_phase(
    phase: Phase,
    accepted: &[&SourceFile],
    precompiled: &[CompiledModule],
) -> Result<Timings, String> {
    let wall_started = Instant::now();
    let process_cpu_started = process_timer_result(ProcessTime::try_now(), "start")?;
    match phase {
        Phase::CompilerCore => {
            let outputs = accepted
                .iter()
                .map(|source| compile(&source.source, &source.filename))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("Rust compiler failed during sample: {error}"))?;
            let timings = finish_timings(wall_started, process_cpu_started)?;
            black_box(&outputs);
            Ok(timings)
        }
        Phase::CompileMarshal => {
            let outputs = accepted
                .iter()
                .map(|source| {
                    compile(&source.source, &source.filename).map(|module| module.marshal())
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("Rust compiler failed during sample: {error}"))?;
            let timings = finish_timings(wall_started, process_cpu_started)?;
            black_box(&outputs);
            Ok(timings)
        }
        Phase::MarshalOnly => {
            let outputs = precompiled
                .iter()
                .map(CompiledModule::marshal)
                .collect::<Vec<_>>();
            let timings = finish_timings(wall_started, process_cpu_started)?;
            black_box(&outputs);
            Ok(timings)
        }
    }
}

fn finish_timings(
    wall_started: Instant,
    process_cpu_started: ProcessTime,
) -> Result<Timings, String> {
    let process_cpu = process_timer_result(process_cpu_started.try_elapsed(), "stop")?;
    let wall = wall_started.elapsed();
    Timings::new(
        duration_ns(wall, "wall-clock")?,
        duration_ns(process_cpu, "process CPU")?,
    )
}

fn process_timer_result<T>(timer: io::Result<T>, action: &str) -> Result<T, String> {
    timer.map_err(|error| format!("failed to {action} process CPU timer: {error}"))
}

fn duration_ns(duration: Duration, clock: &str) -> Result<u64, String> {
    u64::try_from(duration.as_nanos())
        .map_err(|_| format!("{clock} sample duration exceeds u64 nanoseconds"))
}

impl ArtifactIdentity {
    fn capture(python: &str, sources: &[SourceFile]) -> Result<Self, String> {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .map_err(|error| format!("failed to locate the Serc workspace: {error}"))?;
        let object_format = git_output(&workspace, &["rev-parse", "--show-object-format"])?;
        let serc_commit = git_output(&workspace, &["rev-parse", "HEAD^{commit}"])?;
        let status = git_output_bytes(
            &workspace,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?;
        let mut dirty_identity = Vec::new();
        append_hash_record(&mut dirty_identity, &status);
        append_hash_record(
            &mut dirty_identity,
            &git_output_bytes(
                &workspace,
                &["diff", "--binary", "--no-ext-diff", "HEAD", "--", "."],
            )?,
        );
        let untracked = git_output_bytes(
            &workspace,
            &["ls-files", "--others", "--exclude-standard", "-z"],
        )?;
        for relative in untracked
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            let relative = std::str::from_utf8(relative)
                .map_err(|_| "Serc has a non-UTF-8 untracked path".to_string())?;
            append_hash_record(&mut dirty_identity, relative.as_bytes());
            let contents = fs_err::read(workspace.join(relative)).map_err(|error| {
                format!("failed to read untracked file {relative} for identity: {error}")
            })?;
            append_hash_record(&mut dirty_identity, &contents);
        }

        let mut corpus_identity = Vec::new();
        for source in sources {
            append_hash_record(&mut corpus_identity, source.filename.as_bytes());
            append_hash_record(&mut corpus_identity, source.source.as_bytes());
        }

        let benchmark_executable = env::current_exe()
            .and_then(std::fs::canonicalize)
            .map_err(|error| format!("failed to resolve benchmark executable: {error}"))?;
        let profile = profile_from_executable(&benchmark_executable)?;
        let benchmark_contents = fs_err::read(&benchmark_executable).map_err(|error| {
            format!(
                "failed to read benchmark executable {}: {error}",
                benchmark_executable.display()
            )
        })?;
        let python_realpath = python_realpath(python)?;
        let python_contents = fs_err::read(&python_realpath).map_err(|error| {
            format!(
                "failed to read Python executable {}: {error}",
                python_realpath.display()
            )
        })?;
        let cargo_lock = fs_err::read(workspace.join("Cargo.lock"))
            .map_err(|error| format!("failed to read Cargo.lock: {error}"))?;
        let hash_scheme = format!("git-{object_format}-blob");

        Ok(Self {
            profile,
            serc_commit,
            serc_dirty: !status.is_empty(),
            serc_diff_hash: git_hash_bytes(&workspace, &dirty_identity)?,
            cargo_lock_hash: git_hash_bytes(&workspace, &cargo_lock)?,
            corpus_content_hash: git_hash_bytes(&workspace, &corpus_identity)?,
            benchmark_executable: benchmark_executable.display().to_string(),
            benchmark_executable_hash: git_hash_bytes(&workspace, &benchmark_contents)?,
            python_realpath: python_realpath.display().to_string(),
            python_executable_hash: git_hash_bytes(&workspace, &python_contents)?,
            hash_scheme,
        })
    }
}

fn git_output(workspace: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = git_output_bytes(workspace, arguments)?;
    String::from_utf8(output)
        .map(|output| output.trim().to_string())
        .map_err(|error| format!("git output is not UTF-8: {error}"))
}

fn git_output_bytes(workspace: &Path, arguments: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(arguments)
        .output()
        .map_err(|error| format!("failed to run git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed with {}: {}",
            arguments.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    Ok(output.stdout)
}

fn git_hash_bytes(workspace: &Path, contents: &[u8]) -> Result<String, String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to start git hash-object: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "failed to open git hash-object stdin".to_string())?
        .write_all(contents)
        .map_err(|error| format!("failed to hash artifact contents: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("failed to wait for git hash-object: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git hash-object failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    String::from_utf8(output.stdout)
        .map(|output| output.trim().to_string())
        .map_err(|error| format!("git hash-object output is not UTF-8: {error}"))
}

fn append_hash_record(identity: &mut Vec<u8>, value: &[u8]) {
    identity.extend_from_slice(
        &u64::try_from(value.len())
            .expect("artifact identity input length fits in u64")
            .to_be_bytes(),
    );
    identity.extend_from_slice(value);
}

fn profile_from_executable(executable: &Path) -> Result<String, String> {
    let directory = executable
        .parent()
        .ok_or_else(|| "benchmark executable has no parent directory".to_string())?;
    let profile_directory = if directory
        .file_name()
        .is_some_and(|name| name == "examples" || name == "deps")
    {
        directory
            .parent()
            .ok_or_else(|| "benchmark executable has no Cargo profile directory".to_string())?
    } else {
        directory
    };
    profile_directory
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| "Cargo profile directory is not UTF-8".to_string())
}

fn python_realpath(python: &str) -> Result<PathBuf, String> {
    let output = Command::new(python)
        .args([
            "-I",
            "-S",
            "-c",
            "import os, sys; print(os.path.realpath(sys.executable))",
        ])
        .output()
        .map_err(|error| format!("failed to identify {python}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{python} identity command exited with {}",
            output.status
        ));
    }
    let path = String::from_utf8(output.stdout)
        .map_err(|error| format!("{python} executable path is not UTF-8: {error}"))?;
    Ok(PathBuf::from(path.trim()))
}

fn metadata(
    options: &Options,
    identity: &ArtifactIdentity,
    sources: &[SourceFile],
    discovered_source_bytes: usize,
    manifest: &ManifestCounts,
) -> Result<Vec<(String, String)>, String> {
    let command_output = |program: &str, arguments: &[&str]| -> Result<String, String> {
        let output = Command::new(program)
            .args(arguments)
            .output()
            .map_err(|error| format!("failed to run {program}: {error}"))?;
        if !output.status.success() {
            return Err(format!("{program} exited with {}", output.status));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    let mut metadata = vec![
        ("python_requested".to_string(), options.python.clone()),
        (
            "python_realpath".to_string(),
            identity.python_realpath.clone(),
        ),
        (
            "python_executable_hash".to_string(),
            identity.python_executable_hash.clone(),
        ),
        (
            "python_version".to_string(),
            CPYTHON_TARGET.version_string(),
        ),
        (
            "python_magic".to_string(),
            CPYTHON_TARGET.magic_hex(),
        ),
        ("phase".to_string(), options.phase.label().to_string()),
        ("warmups".to_string(), options.warmups.to_string()),
        ("samples".to_string(), options.samples.to_string()),
        cooldown_metadata(options),
        ("schedule".to_string(), "ABBA_BAAB".to_string()),
        (
            "primary_attribution_metric".to_string(),
            "process_cpu_ns".to_string(),
        ),
        (
            "process_cpu_semantics".to_string(),
            "whole-process user-plus-system CPU time".to_string(),
        ),
        (
            "wall_time_role".to_string(),
            "diagnostic".to_string(),
        ),
        ("observations_per_block".to_string(), "4".to_string()),
        (
            "observations_per_engine_per_block".to_string(),
            "2".to_string(),
        ),
        (
            "ratio_contract".to_string(),
            "process_cpu_ns mean_rust_ns/mean_cpython_ns; relative_mad<=0.03; half_delta<=0.03; order_delta<=0.03".to_string(),
        ),
        ("discovered_files".to_string(), sources.len().to_string()),
        (
            "discovered_source_bytes".to_string(),
            discovered_source_bytes.to_string(),
        ),
        (
            "accepted_files".to_string(),
            manifest.accepted.to_string(),
        ),
        (
            "cpython_rejected".to_string(),
            manifest.rejected.to_string(),
        ),
        (
            "accepted_source_bytes".to_string(),
            manifest.source_bytes.to_string(),
        ),
        (
            "exact_marshal_preflight".to_string(),
            "passed".to_string(),
        ),
        (
            "exact_marshal_files".to_string(),
            manifest.accepted.to_string(),
        ),
        ("hash_scheme".to_string(), identity.hash_scheme.clone()),
        (
            "serc_commit".to_string(),
            identity.serc_commit.clone(),
        ),
        (
            "serc_dirty".to_string(),
            identity.serc_dirty.to_string(),
        ),
        (
            "serc_diff_hash".to_string(),
            identity.serc_diff_hash.clone(),
        ),
        (
            "cargo_lock_hash".to_string(),
            identity.cargo_lock_hash.clone(),
        ),
        (
            "corpus_content_hash".to_string(),
            identity.corpus_content_hash.clone(),
        ),
        (
            "benchmark_executable".to_string(),
            identity.benchmark_executable.clone(),
        ),
        (
            "benchmark_executable_hash".to_string(),
            identity.benchmark_executable_hash.clone(),
        ),
        (
            "rustc".to_string(),
            command_output("rustc", &["-Vv"])?.replace('\n', " | "),
        ),
        ("cargo".to_string(), command_output("cargo", &["-V"])?),
        ("os".to_string(), env::consts::OS.to_string()),
        ("arch".to_string(), env::consts::ARCH.to_string()),
        ("profile".to_string(), identity.profile.clone()),
        (
            "target_dir".to_string(),
            env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string()),
        ),
    ];
    for (index, root) in options.roots.iter().enumerate() {
        metadata.push((format!("root_{index}"), root.display().to_string()));
    }
    if let Some(limit) = options.limit {
        metadata.push(("limit".to_string(), limit.to_string()));
    }
    Ok(metadata)
}

fn cooldown_metadata(options: &Options) -> (String, String) {
    (
        "cooldown_ms".to_string(),
        options.cooldown.as_millis().to_string(),
    )
}

fn write_raw(
    options: &Options,
    metadata: &[(String, String)],
    accepted: &[&SourceFile],
    blocks: &[[Timings; 4]],
) -> Result<(), String> {
    let mut text = String::new();
    for (key, value) in metadata {
        writeln!(&mut text, "# metadata\t{key}\t{value}")
            .map_err(|error| format!("failed to format metadata: {error}"))?;
    }
    for source in accepted {
        writeln!(&mut text, "# accepted\t{}", source.filename)
            .map_err(|error| format!("failed to format manifest: {error}"))?;
    }
    text.push_str(
        "phase\tblock\tposition\torder\tkind\tfirst_engine\tengine\tcooldown_ms\twall_elapsed_ns\tprocess_cpu_ns\tfile_count\tsource_bytes\tincluded\n",
    );
    let source_bytes = accepted
        .iter()
        .map(|source| source.source.len())
        .sum::<usize>();
    let (warmups, measured) = blocks.split_at(options.warmups);
    for (kind, included, blocks) in [("warmup", false, warmups), ("measured", true, measured)] {
        for (index, timings) in blocks.iter().enumerate() {
            let block = index + 1;
            let order = block_order(block);
            for (position, (engine, timing)) in order.into_iter().zip(timings).enumerate() {
                writeln!(
                    &mut text,
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    options.phase.label(),
                    block,
                    position + 1,
                    block_order_label(block),
                    kind,
                    order[0].label(),
                    engine.label(),
                    options.cooldown.as_millis(),
                    timing.wall_elapsed_ns,
                    timing.process_cpu_ns,
                    accepted.len(),
                    source_bytes,
                    included,
                )
                .map_err(|error| format!("failed to format sample: {error}"))?;
            }
        }
    }
    fs_err::write(&options.output, text)
        .map_err(|error| format!("failed to write {}: {error}", options.output.display()))
}

fn print_summaries(blocks: &[[Timings; 4]], phase: Phase) -> Result<(), String> {
    for metric in Metric::ALL {
        let blocks = block_samples(blocks, metric);
        let rust = blocks
            .iter()
            .map(|block| block.rust_mean_ns)
            .collect::<Vec<_>>();
        let cpython = blocks
            .iter()
            .map(|block| block.cpython_mean_ns)
            .collect::<Vec<_>>();
        let rust_summary = summarize(&rust)?;
        let cpython_summary = summarize(&cpython)?;
        let ratio_statistics = ratio_statistics(&blocks)?;
        for (engine, summary) in [
            (Engine::Rust, rust_summary),
            (Engine::Cpython, cpython_summary),
        ] {
            println!(
                "metric={} phase={} engine={} median_ns={:.0} p95_ns={} mad_ns={:.0} relative_mad={:.6} half_delta={:.6}",
                metric.label(),
                phase.label(),
                engine.label(),
                summary.median,
                summary.p95.round(),
                summary.mad,
                summary.relative_mad,
                summary.half_delta,
            );
        }
        println!(
            "metric={} phase={} block_ratio_median={:.6} block_ratio_p95={:.6} block_ratio_mad={:.6} block_ratio_relative_mad={:.6} block_ratio_half_delta={:.6} bootstrap95_low={:.6} bootstrap95_high={:.6} abba_median={} baab_median={} order_delta={} stable={}",
            metric.label(),
            phase.label(),
            ratio_statistics.summary.median,
            ratio_statistics.summary.p95,
            ratio_statistics.summary.mad,
            ratio_statistics.summary.relative_mad,
            ratio_statistics.summary.half_delta,
            ratio_statistics.bootstrap_low,
            ratio_statistics.bootstrap_high,
            format_optional(ratio_statistics.abba_median),
            format_optional(ratio_statistics.baab_median),
            format_optional(ratio_statistics.order_delta),
            ratio_statistics.stable,
        );
    }
    Ok(())
}

#[expect(
    clippy::cast_precision_loss,
    reason = "benchmark nanosecond samples are intentionally converted to floating-point means"
)]
fn block_samples(blocks: &[[Timings; 4]], metric: Metric) -> Vec<BlockSample> {
    blocks
        .iter()
        .enumerate()
        .map(|(index, timings)| {
            let [first, second, third, fourth] = timings.map(|timing| metric.value(&timing) as f64);
            let outer_mean = f64::midpoint(first, fourth);
            let inner_mean = f64::midpoint(second, third);
            let (rust_mean_ns, cpython_mean_ns) = if block_order(index + 1)[0] == Engine::Rust {
                (outer_mean, inner_mean)
            } else {
                (inner_mean, outer_mean)
            };
            BlockSample {
                rust_mean_ns,
                cpython_mean_ns,
                ratio: rust_mean_ns / cpython_mean_ns,
            }
        })
        .collect()
}

fn ratio_statistics(blocks: &[BlockSample]) -> Result<RatioStatistics, String> {
    let ratios = blocks.iter().map(|block| block.ratio).collect::<Vec<_>>();
    let summary = summarize(&ratios)?;
    let (bootstrap_low, bootstrap_high) =
        bootstrap_median_ci(&ratios, BOOTSTRAP_SAMPLES, BOOTSTRAP_SEED)?;
    let abba = blocks
        .iter()
        .step_by(2)
        .map(|block| block.ratio)
        .collect::<Vec<_>>();
    let baab = blocks
        .iter()
        .skip(1)
        .step_by(2)
        .map(|block| block.ratio)
        .collect::<Vec<_>>();
    let abba_median = (!abba.is_empty()).then(|| median_f64(&abba)).transpose()?;
    let baab_median = (!baab.is_empty()).then(|| median_f64(&baab)).transpose()?;
    let order_delta = abba_median
        .zip(baab_median)
        .map(|(abba, baab)| relative_difference(abba, baab));
    let stable = summary.relative_mad <= 0.03
        && summary.half_delta <= 0.03
        && order_delta.is_some_and(|delta| delta <= 0.03);
    Ok(RatioStatistics {
        summary,
        bootstrap_low,
        bootstrap_high,
        abba_median,
        baab_median,
        order_delta,
        stable,
    })
}

fn summarize(values: &[f64]) -> Result<Summary, String> {
    if values.is_empty() {
        return Err("cannot summarize an empty sample".to_string());
    }
    let median = median_f64(values)?;
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let p95_index = ((95 * sorted.len()).div_ceil(100)).saturating_sub(1);
    let p95 = sorted[p95_index];
    let deviations = values
        .iter()
        .map(|value| (*value - median).abs())
        .collect::<Vec<_>>();
    let mad = median_f64(&deviations)?;
    let (first, second) = if values.len() == 1 {
        let value = values[0];
        (value, value)
    } else {
        let middle = values.len().div_ceil(2);
        (
            median_f64(&values[..middle])?,
            median_f64(&values[middle..])?,
        )
    };
    let half_delta = relative_difference(first, second);
    Ok(Summary {
        median,
        p95,
        mad,
        relative_mad: mad / median,
        half_delta,
    })
}

fn relative_difference(left: f64, right: f64) -> f64 {
    if left == 0.0 || right == 0.0 {
        0.0
    } else {
        left.max(right) / left.min(right) - 1.0
    }
}

fn format_optional(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_string(), |value| format!("{value:.6}"))
}

fn median_f64(values: &[f64]) -> Result<f64, String> {
    if values.is_empty() {
        return Err("cannot compute median of an empty sample".to_string());
    }
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        Ok(f64::midpoint(values[middle - 1], values[middle]))
    } else {
        Ok(values[middle])
    }
}

fn bootstrap_median_ci(
    ratios: &[f64],
    repetitions: usize,
    seed: u64,
) -> Result<(f64, f64), String> {
    if ratios.is_empty() || repetitions == 0 {
        return Err("bootstrap requires ratios and repetitions".to_string());
    }
    let mut state = seed;
    let mut medians = Vec::with_capacity(repetitions);
    let mut sample = Vec::with_capacity(ratios.len());
    for _ in 0..repetitions {
        sample.clear();
        for _ in 0..ratios.len() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let length = u64::try_from(ratios.len()).expect("sample length fits in u64");
            let index = usize::try_from(state % length).expect("index is below sample length");
            sample.push(ratios[index]);
        }
        medians.push(median_f64(&sample)?);
    }
    medians.sort_by(f64::total_cmp);
    let low = ((repetitions * 25).div_ceil(1000)).saturating_sub(1);
    let high = ((repetitions * 975).div_ceil(1000)).saturating_sub(1);
    Ok((medians[low], medians[high]))
}

#[cfg(test)]
mod tests {
    use clap::{Parser, error::ErrorKind};
    use std::io;
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{
        AcceptedFile, BOOTSTRAP_SEED, BlockSample, Engine, Metric, Options, Phase, PythonRunner,
        SourceFile, Timings, block_order, block_samples, bootstrap_median_ci, cooldown_metadata,
        discover, expected_row_count, parse_sample_response, process_timer_result,
        profile_from_executable, ratio_statistics, summarize, verify_exact_marshal, write_raw,
    };

    #[test]
    fn python_helper_rejects_prerelease_interpreters() {
        let python = std::env::var("SERC_TEST_PYTHON").unwrap_or_else(|_| "python3.14".to_string());
        let script = "import collections, sys; Version = collections.namedtuple('Version', 'major minor micro releaselevel serial'); sys.version_info = Version(3, 14, 5, 'candidate', 1); exec(sys.argv.pop(1))";
        let output = std::process::Command::new(python)
            .args([
                "-I",
                "-S",
                "-c",
                script,
                super::PYTHON_HELPER,
                serc::CPYTHON_TARGET.implementation,
                &serc::CPYTHON_TARGET.version_string(),
                &serc::CPYTHON_TARGET.magic_hex(),
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stdout).contains("candidate 1"));
    }

    #[test]
    fn python_helper_benchmarks_only_accepted_sources() {
        const CHILD: &str = "SERC_BENCHMARK_PROTOCOL_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // Isolate the pipe exchange so a regression fails within a deadline
            // instead of hanging the entire test suite.
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::python_helper_benchmarks_only_accepted_sources",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success(), "CPython protocol test failed: {status}");
                    return;
                }
                if std::time::Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("CPython protocol test exceeded 30 seconds");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        let python = std::env::var("SERC_TEST_PYTHON").unwrap_or_else(|_| "python3.14".to_string());
        let sources = [
            (
                "accepted.py",
                format!("answer = '{}'\n", "caf\u{e9}".repeat(65_536)),
            ),
            ("syntax.py", "if:\n".to_string()),
            ("semantic.py", "return\n".to_string()),
            (
                "after_rejections.py",
                format!(
                    "raise RuntimeError('{}')\n",
                    "compiled, never executed".repeat(8_192)
                ),
            ),
        ]
        .map(|(filename, source)| SourceFile {
            path: PathBuf::from(filename),
            filename: filename.to_string(),
            source,
        });
        let mut runner = PythonRunner::start(&python).unwrap();
        let manifest = runner.initialize(&sources).unwrap();
        let samples = [
            Phase::CompilerCore,
            Phase::CompileMarshal,
            Phase::MarshalOnly,
        ]
        .map(|phase| runner.run_phase(phase).unwrap());
        runner.finish().unwrap();

        assert_eq!(manifest.rejected, 2);
        assert_eq!(
            manifest.source_bytes,
            sources[0].source.len() + sources[3].source.len()
        );
        let expected_files = [0, 3].map(|source_index| AcceptedFile {
            source_index,
            marshaled: serc::compile(
                &sources[source_index].source,
                &sources[source_index].filename,
            )
            .unwrap()
            .marshal(),
        });
        assert_eq!(manifest.accepted, expected_files);
        for sample in samples {
            assert_eq!(sample.count, 2);
            assert!(sample.observed > 0);
        }
        let marshaled_bytes = expected_files
            .iter()
            .map(|file| file.marshaled.len())
            .sum::<usize>();
        assert_eq!(samples[1].observed, marshaled_bytes);
        assert_eq!(samples[2].observed, marshaled_bytes);
    }

    #[test]
    fn parses_required_options_and_limit() {
        let options = Options::try_parse_from([
            "benchmark_cpython_corpus",
            "--python",
            "/opt/homebrew/bin/python3",
            "--phase",
            "compiler_core",
            "--warmups",
            "5",
            "--samples",
            "21",
            "--cooldown-ms",
            "5000",
            "--output",
            "samples.tsv",
            "--limit",
            "10",
            "corpus",
        ])
        .expect("valid options");
        assert_eq!(options.phase, Phase::CompilerCore);
        assert_eq!(options.warmups, 5);
        assert_eq!(options.samples, 21);
        assert_eq!(options.cooldown, Duration::from_millis(5000));
        assert_eq!(options.limit, Some(10));
        assert_eq!(options.roots.len(), 1);
    }

    #[test]
    fn corpus_discovery_fails_closed() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let missing = directory.path().join("missing");

        let error = discover(std::slice::from_ref(&missing), None)
            .expect_err("a missing corpus root must fail");

        assert!(error.contains("failed to walk corpus root"));
        assert!(error.contains(&missing.display().to_string()));
    }

    #[test]
    fn exact_marshal_preflight_rejects_mismatches() {
        assert_eq!(
            verify_exact_marshal(&[], &[], &[]),
            Err("exact-marshal preflight has no CPython-accepted files".to_string())
        );

        let source = SourceFile {
            path: PathBuf::from("source.py"),
            filename: "source.py".to_string(),
            source: "pass\n".to_string(),
        };
        let module = serc::compile(&source.source, &source.filename).expect("compile source");
        let mut expected = AcceptedFile {
            source_index: 0,
            marshaled: module.marshal(),
        };
        verify_exact_marshal(
            &[&source],
            std::slice::from_ref(&module),
            std::slice::from_ref(&expected),
        )
        .expect("identical marshal output");

        expected.marshaled[0] ^= 1;
        let error = verify_exact_marshal(&[&source], &[module], &[expected])
            .expect_err("different marshal output must fail");
        assert!(error.contains("exact-marshal preflight failed for source.py at byte 0"));
    }

    #[test]
    fn derives_the_actual_cargo_profile_from_the_executable() {
        assert_eq!(
            profile_from_executable(PathBuf::from("target/profiling/examples/benchmark").as_path()),
            Ok("profiling".to_string())
        );
        assert_eq!(
            profile_from_executable(PathBuf::from("target/debug/deps/benchmark-hash").as_path()),
            Ok("debug".to_string())
        );
    }

    #[test]
    fn parses_each_supported_phase() {
        for (label, phase) in [
            ("compiler_core", Phase::CompilerCore),
            ("compile_marshal", Phase::CompileMarshal),
            ("marshal_only", Phase::MarshalOnly),
        ] {
            let options = Options::try_parse_from([
                "benchmark_cpython_corpus",
                "--python",
                "python3",
                "--phase",
                label,
                "--warmups",
                "1",
                "--samples",
                "1",
                "--cooldown-ms",
                "0",
                "--output",
                "samples.tsv",
                "corpus",
            ])
            .expect("supported phase");
            assert_eq!(options.phase, phase);
        }
    }

    #[test]
    fn requires_a_supported_phase() {
        let missing = Options::try_parse_from([
            "benchmark_cpython_corpus",
            "--python",
            "python3",
            "--warmups",
            "1",
            "--samples",
            "1",
            "--cooldown-ms",
            "0",
            "--output",
            "samples.tsv",
            "corpus",
        ])
        .expect_err("phase is required");
        assert_eq!(missing.kind(), ErrorKind::MissingRequiredArgument);
        assert!(missing.to_string().contains("--phase"));

        let unsupported = Options::try_parse_from([
            "benchmark_cpython_corpus",
            "--python",
            "python3",
            "--phase",
            "all",
            "--warmups",
            "1",
            "--samples",
            "1",
            "--cooldown-ms",
            "0",
            "--output",
            "samples.tsv",
            "corpus",
        ])
        .expect_err("unsupported phase must fail");
        assert_eq!(unsupported.kind(), ErrorKind::InvalidValue);
    }

    #[test]
    fn rejects_zero_iteration_counts() {
        for (warmups, samples, limit) in [("0", "1", "1"), ("1", "0", "1"), ("1", "1", "0")] {
            let error = Options::try_parse_from([
                "benchmark_cpython_corpus",
                "--python",
                "python3",
                "--phase",
                "compiler_core",
                "--warmups",
                warmups,
                "--samples",
                samples,
                "--limit",
                limit,
                "--cooldown-ms",
                "0",
                "--output",
                "samples.tsv",
                "corpus",
            ])
            .expect_err("zero iteration counts must fail");
            assert_eq!(error.kind(), ErrorKind::ValueValidation);
        }
    }

    #[test]
    fn requires_cooldown() {
        let error = Options::try_parse_from([
            "benchmark_cpython_corpus",
            "--python",
            "python3",
            "--phase",
            "compiler_core",
            "--warmups",
            "1",
            "--samples",
            "1",
            "--output",
            "samples.tsv",
            "corpus",
        ])
        .expect_err("cooldown is required");
        assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
        assert!(error.to_string().contains("--cooldown-ms"));
    }

    #[test]
    fn records_cooldown_metadata() {
        let options = Options {
            python: "python3".to_string(),
            phase: Phase::CompilerCore,
            warmups: 1,
            samples: 1,
            cooldown: Duration::from_millis(5000),
            output: PathBuf::from("samples.tsv"),
            roots: vec![PathBuf::from("corpus")],
            limit: None,
        };
        assert_eq!(
            cooldown_metadata(&options),
            ("cooldown_ms".to_string(), "5000".to_string())
        );
    }

    #[test]
    fn counts_rows_for_exactly_one_phase() {
        let options = Options {
            python: "python3".to_string(),
            phase: Phase::MarshalOnly,
            warmups: 5,
            samples: 21,
            cooldown: Duration::ZERO,
            output: PathBuf::from("samples.tsv"),
            roots: vec![PathBuf::from("corpus")],
            limit: None,
        };
        assert_eq!(expected_row_count(&options), Ok(104));
    }

    #[test]
    fn parses_dual_clock_response() {
        let sample =
            parse_sample_response("RESULT\t120\t90\t2\t42").expect("valid dual-clock response");
        assert_eq!(
            sample.timings,
            Timings {
                wall_elapsed_ns: 120,
                process_cpu_ns: 90,
            }
        );
        assert_eq!(sample.count, 2);
        assert_eq!(sample.observed, 42);

        let error = parse_sample_response("RESULT\t120\t2\t42")
            .expect_err("response without process CPU time must fail");
        assert!(error.contains("invalid CPython sample response"));
    }

    #[test]
    fn propagates_process_timer_errors() {
        let error =
            process_timer_result::<Duration>(Err(io::Error::other("clock unavailable")), "stop")
                .expect_err("timer error must propagate");
        assert_eq!(error, "failed to stop process CPU timer: clock unavailable");
    }

    #[test]
    fn summarizes_nearest_rank_p95_and_half_drift() {
        let summary = summarize(&[
            100.0, 101.0, 102.0, 103.0, 104.0, 105.0, 106.0, 107.0, 200.0, 201.0, 202.0,
        ])
        .expect("non-empty sample");
        assert_eq!(summary.median, 105.0);
        assert_eq!(summary.p95, 202.0);
        assert!(summary.mad > 0.0);
        assert!(summary.half_delta > 0.5);
    }

    #[test]
    fn alternates_abba_and_baab_blocks() {
        assert_eq!(
            block_order(1),
            [Engine::Rust, Engine::Cpython, Engine::Cpython, Engine::Rust]
        );
        assert_eq!(
            block_order(2),
            [Engine::Cpython, Engine::Rust, Engine::Rust, Engine::Cpython]
        );
    }

    #[test]
    fn aggregates_two_observations_per_engine() {
        let blocks = [
            [(100, 70), (80, 60), (100, 70), (120, 90)],
            [(80, 60), (100, 70), (120, 90), (100, 70)],
        ]
        .map(|block| block.map(|(wall, cpu)| Timings::new(wall, cpu).unwrap()));
        let wall = block_samples(&blocks, Metric::Wall);
        assert_eq!(wall.len(), 2);
        for block in &wall {
            assert_eq!(block.rust_mean_ns, 110.0);
            assert_eq!(block.cpython_mean_ns, 90.0);
            assert_eq!(block.ratio, 110.0 / 90.0);
        }
        let process_cpu = block_samples(&blocks, Metric::ProcessCpu);
        for block in &process_cpu {
            assert_eq!(block.rust_mean_ns, 80.0);
            assert_eq!(block.cpython_mean_ns, 65.0);
            assert_eq!(block.ratio, 80.0 / 65.0);
        }
    }

    #[test]
    fn checks_ratio_drift_and_order_strata() {
        let stable = (1..=21)
            .map(|block| {
                let ratio = if block % 2 == 1 { 1.001 } else { 0.999 };
                BlockSample {
                    rust_mean_ns: ratio * 100.0,
                    cpython_mean_ns: 100.0,
                    ratio,
                }
            })
            .collect::<Vec<_>>();
        let statistics = ratio_statistics(&stable).expect("valid ratio statistics");
        assert!(statistics.stable);
        assert!(statistics.summary.relative_mad <= 0.03);
        assert!(statistics.summary.half_delta <= 0.03);
        assert!(statistics.order_delta.is_some_and(|delta| delta <= 0.03));

        let unstable = (1..=21)
            .map(|block| {
                let ratio = if block % 2 == 1 { 1.0 } else { 1.1 };
                BlockSample {
                    rust_mean_ns: ratio * 100.0,
                    cpython_mean_ns: 100.0,
                    ratio,
                }
            })
            .collect::<Vec<_>>();
        assert!(
            !ratio_statistics(&unstable)
                .expect("valid ratio statistics")
                .stable
        );
    }

    #[test]
    fn bootstrap_is_deterministic() {
        let ratios = [0.8, 0.9, 1.0, 1.1, 1.2];
        let first =
            bootstrap_median_ci(&ratios, 1_000, BOOTSTRAP_SEED).expect("valid bootstrap sample");
        let second =
            bootstrap_median_ci(&ratios, 1_000, BOOTSTRAP_SEED).expect("valid bootstrap sample");
        assert_eq!(first, second);
        assert!(first.0 <= 1.0 && first.1 >= 1.0);
    }

    #[test]
    fn serializes_warmups_and_both_block_orders() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let options = Options {
            python: "python3".to_string(),
            phase: Phase::CompilerCore,
            warmups: 1,
            samples: 2,
            cooldown: Duration::from_millis(5000),
            output: directory.path().join("samples.tsv"),
            roots: vec![PathBuf::from("corpus")],
            limit: None,
        };
        let source = SourceFile {
            path: PathBuf::from("source.py"),
            filename: "source.py".to_string(),
            source: "pass\n".to_string(),
        };
        let blocks = [
            [(10, 5); 4],
            [(120, 90), (80, 60), (100, 70), (140, 100)],
            [(200, 150), (210, 160), (220, 170), (230, 180)],
        ]
        .map(|block| block.map(|(wall, cpu)| Timings::new(wall, cpu).unwrap()));
        write_raw(
            &options,
            &[(
                "primary_attribution_metric".to_string(),
                "process_cpu_ns".to_string(),
            )],
            &[&source],
            &blocks,
        )
        .expect("serialize raw observations");
        let text = fs_err::read_to_string(&options.output).expect("read raw observations");
        assert_eq!(
            text,
            concat!(
                "# metadata\tprimary_attribution_metric\tprocess_cpu_ns\n",
                "# accepted\tsource.py\n",
                "phase\tblock\tposition\torder\tkind\tfirst_engine\tengine\t",
                "cooldown_ms\twall_elapsed_ns\tprocess_cpu_ns\tfile_count\t",
                "source_bytes\tincluded\n",
                "compiler_core\t1\t1\tABBA\twarmup\trust\trust\t5000\t10\t5\t1\t5\tfalse\n",
                "compiler_core\t1\t2\tABBA\twarmup\trust\tcpython\t5000\t10\t5\t1\t5\tfalse\n",
                "compiler_core\t1\t3\tABBA\twarmup\trust\tcpython\t5000\t10\t5\t1\t5\tfalse\n",
                "compiler_core\t1\t4\tABBA\twarmup\trust\trust\t5000\t10\t5\t1\t5\tfalse\n",
                "compiler_core\t1\t1\tABBA\tmeasured\trust\trust\t5000\t120\t90\t1\t5\ttrue\n",
                "compiler_core\t1\t2\tABBA\tmeasured\trust\tcpython\t5000\t80\t60\t1\t5\ttrue\n",
                "compiler_core\t1\t3\tABBA\tmeasured\trust\tcpython\t5000\t100\t70\t1\t5\ttrue\n",
                "compiler_core\t1\t4\tABBA\tmeasured\trust\trust\t5000\t140\t100\t1\t5\ttrue\n",
                "compiler_core\t2\t1\tBAAB\tmeasured\tcpython\tcpython\t5000\t200\t150\t1\t5\ttrue\n",
                "compiler_core\t2\t2\tBAAB\tmeasured\tcpython\trust\t5000\t210\t160\t1\t5\ttrue\n",
                "compiler_core\t2\t3\tBAAB\tmeasured\tcpython\trust\t5000\t220\t170\t1\t5\ttrue\n",
                "compiler_core\t2\t4\tBAAB\tmeasured\tcpython\tcpython\t5000\t230\t180\t1\t5\ttrue\n",
            )
        );
    }
}
