use std::collections::HashMap;
use std::ffi::OsString;
use std::hint::black_box;
use std::io::{self, Read, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::builder::Styles;
use clap::builder::styling::{AnsiColor, Effects};
use clap::{Parser, Subcommand, ValueEnum, ValueHint};
use rayon::prelude::*;
use serc::{
    CompileError, CompileOptions, NativeBuildProfile, OptimizationLevel, PythonVersion,
    compile_bytes_with_path_and_options,
};
use walkdir::WalkDir;

const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Cyan.on_default());

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    name = "serc",
    about = "Compile Python source to CPython bytecode.",
    arg_required_else_help = true,
    styles = STYLES
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compile Python files to .pyc files.
    Compile(CompileCommand),
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum InvalidationMode {
    /// Validate against source modification time and size.
    Timestamp,
    /// Validate against a hash of the source contents.
    CheckedHash,
    /// Trust an external system to keep the cache current.
    UncheckedHash,
}

impl InvalidationMode {
    fn flags(self) -> u32 {
        match self {
            Self::Timestamp => 0,
            Self::CheckedHash => 3,
            Self::UncheckedHash => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, clap::Args)]
struct CompileCommand {
    /// Python files or directories to compile [default: .]
    #[arg(value_hint = ValueHint::AnyPath)]
    paths: Vec<PathBuf>,

    /// Read file paths from a newline-delimited manifest, or standard input with -.
    #[arg(long, value_name = "FILE", conflicts_with = "paths")]
    files_from: Option<PathBuf>,

    /// Separate manifest paths with NUL bytes instead of newlines.
    #[arg(long, requires = "files_from")]
    null: bool,

    /// Python language and bytecode version to target (3.12, 3.13, 3.14, or 3.15).
    #[arg(long, default_value_t = PythonVersion::default())]
    python_version: PythonVersion,

    /// Floating-point instruction ordering for a named CPython native build.
    #[arg(long, default_value_t = NativeBuildProfile::default())]
    native_build_profile: NativeBuildProfile,

    /// How Python validates cached bytecode against its source.
    #[arg(long, value_enum, default_value = "timestamp")]
    invalidation_mode: InvalidationMode,

    /// Remove assertions and set __debug__ to false; repeat to also remove docstrings.
    // Clap's Count action caps occurrences at 255, but the cache tag must retain them all.
    #[arg(short = 'O', action = clap::ArgAction::Append, num_args = 0, default_missing_value = "true", conflicts_with = "optimize")]
    optimization_flags: Vec<bool>,

    /// Python optimization level: 0 (default), 1 (-O), or 2 (-OO).
    #[arg(long, value_name = "LEVEL")]
    optimize: Option<OptimizationLevel>,

    /// Maximum number of files to compile in parallel [default: available CPUs].
    #[arg(short, long)]
    jobs: Option<NonZeroUsize>,

    /// Compile the sources without writing .pyc files.
    #[arg(long)]
    check: bool,

    /// Recompile even when the existing cache is current.
    ///
    /// Cache headers do not identify native build profiles. Use this option to regenerate
    /// bytecode after changing the compiler or native build profile.
    #[arg(short, long)]
    force: bool,

    /// Format of the success summary.
    #[arg(long, value_enum, default_value = "text")]
    output_format: OutputFormat,

    /// Suppress the success summary.
    #[arg(short, long)]
    quiet: bool,
}

pub(crate) fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(exit_code) => exit_code,
        Err(message) => {
            write_error(&message);
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, String> {
    match cli.command {
        Command::Compile(command) => compile_command(command),
    }
}

fn compile_command(command: CompileCommand) -> Result<ExitCode, String> {
    let (paths, source_sizes) = if let Some(manifest) = &command.files_from {
        let (paths, source_sizes) = read_file_list(manifest, command.null)?;
        (paths, Some(source_sizes))
    } else {
        let roots = if command.paths.is_empty() {
            vec![PathBuf::from(".")]
        } else {
            command.paths
        };
        (discover(&roots)?, None)
    };
    let mut failures = 0_usize;
    let skipped = AtomicUsize::new(0);
    // Code generation has three levels, but Python keeps every `-O` in the cache tag.
    let cache_optimization = command
        .optimize
        .map_or(command.optimization_flags.len(), |level| match level {
            OptimizationLevel::Zero => 0,
            OptimizationLevel::One => 1,
            OptimizationLevel::Two => 2,
        });
    let options = CompileOptions {
        python_version: command.python_version,
        native_build_profile: command.native_build_profile,
        optimization_level: match cache_optimization {
            0 => OptimizationLevel::Zero,
            1 => OptimizationLevel::One,
            _ => OptimizationLevel::Two,
        },
        ..CompileOptions::default()
    };

    let (jobs, output_groups) = prepare_compilation(
        &paths,
        options.python_version,
        cache_optimization,
        command.check,
        jobs_for_inputs(command.jobs, paths.len()),
    );
    if jobs <= 1 {
        for path in &paths {
            match compile_path(
                path,
                command.check,
                command.force,
                command.invalidation_mode,
                &options,
                cache_optimization,
            ) {
                Ok(true) => {}
                Ok(false) => {
                    skipped.fetch_add(1, Ordering::Relaxed);
                }
                Err(message) => {
                    failures += 1;
                    write_error(&message);
                }
            }
        }
    } else {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            // Nested Python expressions and scopes use recursive compiler traversals.
            .stack_size(8 * 1024 * 1024)
            .build()
            .map_err(|error| format!("failed to start compilation workers: {error}"))?;
        let mut errors: Vec<(usize, String)> = pool.install(|| {
            let groups = output_groups.unwrap_or_else(|| {
                paths
                    .iter()
                    .enumerate()
                    .map(|(index, path)| vec![(index, path.as_path())])
                    .collect()
            });
            let groups = prioritize_compilation_groups(groups, jobs, source_sizes.as_deref());
            let next_group = AtomicUsize::new(0);
            (0..jobs.min(groups.len()))
                .into_par_iter()
                .with_max_len(1)
                .flat_map_iter(|_| {
                    // Each worker takes the largest remaining group. Splitting the sorted slice
                    // into Rayon chunks would strand some large inputs behind a worker's backlog.
                    std::iter::from_fn(|| groups.get(next_group.fetch_add(1, Ordering::Relaxed)))
                        .flatten()
                        .filter_map(|&(index, path)| {
                            match compile_path(
                                path,
                                command.check,
                                command.force,
                                command.invalidation_mode,
                                &options,
                                cache_optimization,
                            ) {
                                Ok(true) => None,
                                Ok(false) => {
                                    skipped.fetch_add(1, Ordering::Relaxed);
                                    None
                                }
                                Err(message) => Some((index, message)),
                            }
                        })
                })
                .collect()
        });
        errors.sort_unstable_by_key(|(index, _)| *index);
        failures = errors.len();
        for (_, message) in errors {
            write_error(&message);
        }
    }

    if failures > 0 {
        write_error(&format!(
            "failed to compile {failures} of {} files",
            paths.len()
        ));
        return Ok(ExitCode::FAILURE);
    }

    if !command.quiet {
        let skipped = skipped.load(Ordering::Relaxed);
        let compiled = paths.len() - skipped;
        let noun = if compiled == 1 { "file" } else { "files" };
        let mut stdout = io::stdout().lock();
        match command.output_format {
            OutputFormat::Json => {
                writeln!(stdout, "{{\"compiled\":{compiled},\"skipped\":{skipped}}}")
            }
            OutputFormat::Text if skipped == 0 => writeln!(stdout, "Compiled {compiled} {noun}"),
            OutputFormat::Text => {
                writeln!(stdout, "Compiled {compiled} {noun}, skipped {skipped}")
            }
        }
        .map_err(|error| format!("failed to write to stdout: {error}"))?;
    }

    Ok(ExitCode::SUCCESS)
}

/// Read exact file paths without applying directory discovery or a default input.
///
/// Retain validated sizes as scheduling hints; workers still load fresh metadata before reading.
fn read_file_list(manifest: &Path, null: bool) -> Result<(Vec<PathBuf>, Vec<u64>), String> {
    let mut contents = Vec::new();
    if manifest == Path::new("-") {
        io::stdin()
            .lock()
            .read_to_end(&mut contents)
            .map_err(|error| format!("failed to read file paths from stdin: {error}"))?;
    } else {
        contents = fs_err::read(manifest)
            .map_err(|error| format!("failed to read {}: {error}", manifest.display()))?;
    }
    let separator = if null { b'\0' } else { b'\n' };
    let mut paths = Vec::new();
    for path in contents.split(|&byte| byte == separator) {
        let path = if null {
            path
        } else {
            path.strip_suffix(b"\r").unwrap_or(path)
        };
        if path.is_empty() {
            continue;
        }
        #[cfg(unix)]
        let path = {
            use std::os::unix::ffi::OsStrExt;
            PathBuf::from(std::ffi::OsStr::from_bytes(path))
        };
        #[cfg(not(unix))]
        let path = PathBuf::from(
            std::str::from_utf8(path)
                .map_err(|error| format!("file paths must be UTF-8 on this platform: {error}"))?,
        );
        let metadata = fs_err::metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if !metadata.is_file() {
            return Err(format!("{} is not a file", path.display()));
        }
        paths.push((path, metadata.len()));
    }
    paths.sort_by(|(left, _), (right, _)| left.cmp(right));
    paths.dedup_by(|(left, _), (right, _)| left == right);
    Ok(paths.into_iter().unzip())
}

/// Follow Rayon's environment and CPU defaults without starting idle workers for small inputs.
fn jobs_for_inputs(requested: Option<NonZeroUsize>, inputs: usize) -> usize {
    if inputs <= 1 {
        return inputs;
    }
    requested
        .map_or_else(
            || {
                let configured = std::env::var("RAYON_NUM_THREADS").ok();
                let legacy = std::env::var("RAYON_RS_NUM_CPUS").ok();
                default_jobs(configured.as_deref(), legacy.as_deref(), || {
                    std::thread::available_parallelism().map_or(1, NonZeroUsize::get)
                })
            },
            NonZeroUsize::get,
        )
        .min(inputs)
}

fn default_jobs(
    configured: Option<&str>,
    legacy: Option<&str>,
    available: impl FnOnce() -> usize,
) -> usize {
    match configured.and_then(|value| value.parse::<usize>().ok()) {
        Some(0) => available(),
        Some(jobs) => jobs,
        None => legacy
            .and_then(|value| value.parse::<NonZeroUsize>().ok())
            .map_or_else(available, NonZeroUsize::get),
    }
}

type CompilationGroup<'a> = Vec<(usize, &'a Path)>;

/// Limit cache-writing workers to groups that can make progress independently.
fn prepare_compilation(
    paths: &[PathBuf],
    python_version: PythonVersion,
    cache_optimization: usize,
    check: bool,
    jobs: usize,
) -> (usize, Option<Vec<CompilationGroup<'_>>>) {
    if check || jobs <= 1 {
        return (jobs, None);
    }
    let groups = compilation_groups(paths, python_version, cache_optimization);
    (jobs.min(groups.len()), Some(groups))
}

/// Start the most expensive independent groups first, retaining order within aliased outputs.
fn prioritize_compilation_groups<'a>(
    mut groups: Vec<CompilationGroup<'a>>,
    jobs: usize,
    source_sizes: Option<&[u64]>,
) -> Vec<CompilationGroup<'a>> {
    if groups.len() > jobs {
        groups.sort_by_cached_key(|group| {
            let size = group.iter().fold(0_u64, |total, (index, path)| {
                total.saturating_add(source_sizes.map_or_else(
                    || fs_err::metadata(path).map_or(0, |metadata| metadata.len()),
                    |sizes| sizes[*index],
                ))
            });
            (std::cmp::Reverse(size), group[0].0)
        });
    }
    groups
}

/// Keep inputs that share an output in discovery order so the same cache wins as in serial mode.
fn compilation_groups(
    paths: &[PathBuf],
    python_version: PythonVersion,
    cache_optimization: usize,
) -> Vec<CompilationGroup<'_>> {
    use std::collections::HashSet;
    use std::sync::Arc;

    let mut directories = HashMap::new();
    let mut non_ascii_directories = HashSet::new();
    let mut unknown_directory = false;
    let mut identities = Vec::with_capacity(paths.len());
    for path in paths {
        let output = cache_path(path, python_version, cache_optimization)
            .ok()
            .and_then(|output| {
                let parent = output.parent()?;
                let filename = output.file_name()?;
                let directory = if let Some(directory) = directories.get(parent) {
                    Arc::clone(directory)
                } else {
                    let directory = Arc::new(cache_directory_identity(parent));
                    directories.insert(parent.to_owned(), Arc::clone(&directory));
                    directory
                };
                unknown_directory |= matches!(directory.as_ref(), CacheDirectoryIdentity::Unknown);
                if !filename.as_encoded_bytes().is_ascii() {
                    non_ascii_directories.insert(Arc::clone(&directory));
                }
                Some((directory, filename.to_ascii_lowercase()))
            });
        identities.push(output);
    }

    let mut groups = Vec::<Vec<_>>::new();
    let mut outputs = HashMap::new();
    for (index, (path, output)) in paths.iter().zip(identities).enumerate() {
        let output = output.map(|(directory, filename)| {
            // Filesystems disagree on Unicode normalization and case folding, including aliases
            // between Unicode and ASCII names. Serialize the directory when either may occur.
            if unknown_directory {
                // An unidentified directory may alias one that was identified successfully.
                (None, non_ascii_directories.is_empty().then_some(filename))
            } else {
                let filename = (!non_ascii_directories.contains(&directory)).then_some(filename);
                (Some(directory), filename)
            }
        });
        let group = output.map_or(groups.len(), |output| {
            *outputs.entry(output).or_insert(groups.len())
        });
        if group == groups.len() {
            groups.push(Vec::new());
        }
        groups[group].push((index, path.as_path()));
    }
    groups
}

#[derive(Eq, Hash, PartialEq)]
enum CacheDirectoryIdentity {
    Existing(DirectoryHandle),
    Parent(DirectoryHandle),
    Unknown,
}

#[cfg(unix)]
#[derive(Eq, Hash, PartialEq)]
struct DirectoryHandle {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
impl DirectoryHandle {
    fn from_path(path: &Path) -> io::Result<Self> {
        use std::os::unix::fs::MetadataExt;

        let metadata = fs_err::metadata(path)?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

#[cfg(not(unix))]
type DirectoryHandle = same_file::Handle;

/// Identify directory aliases even when the cache is absent or spelled with different casing.
fn cache_directory_identity(directory: &Path) -> CacheDirectoryIdentity {
    if let Ok(handle) = DirectoryHandle::from_path(directory) {
        return CacheDirectoryIdentity::Existing(handle);
    }
    let parent = directory
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    DirectoryHandle::from_path(parent)
        .map(CacheDirectoryIdentity::Parent)
        .unwrap_or(CacheDirectoryIdentity::Unknown)
}

fn discover(roots: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();

    for root in roots {
        let metadata = fs_err::metadata(root)
            .map_err(|error| format!("failed to inspect {}: {error}", root.display()))?;
        if metadata.is_file() {
            paths.push(root.clone());
            continue;
        }
        if !metadata.is_dir() {
            return Err(format!(
                "{} is neither a file nor a directory",
                root.display()
            ));
        }

        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| entry.depth() == 0 || entry.file_name() != "__pycache__")
        {
            let entry =
                entry.map_err(|error| format!("failed to walk {}: {error}", root.display()))?;
            if (entry.file_type().is_file()
                || (entry.file_type().is_symlink() && entry.path().is_file()))
                && has_python_suffix(entry.path())
            {
                paths.push(entry.into_path());
            }
        }
    }

    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err("no Python files found".to_string());
    }
    Ok(paths)
}

fn has_python_suffix(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.as_encoded_bytes().ends_with(b".py"))
}

fn compile_path(
    path: &Path,
    check: bool,
    force: bool,
    invalidation_mode: InvalidationMode,
    options: &CompileOptions,
    cache_optimization: usize,
) -> Result<bool, String> {
    compile_path_with_metadata(
        path,
        check,
        force,
        invalidation_mode,
        options,
        cache_optimization,
        |path| fs_err::metadata(path),
    )
}

/// Compile a source file, returning false when its existing cache is current.
///
/// Check mode skips both the metadata loader and cache output.
fn compile_path_with_metadata(
    path: &Path,
    check: bool,
    force: bool,
    invalidation_mode: InvalidationMode,
    options: &CompileOptions,
    cache_optimization: usize,
    load_metadata: impl FnOnce(&Path) -> io::Result<std::fs::Metadata>,
) -> Result<bool, String> {
    // Snapshot timestamp-cache metadata before reading. If the path changes during compilation,
    // the resulting header describes the old state and cannot validate the new source as fresh.
    let metadata = (!check)
        .then(|| {
            load_metadata(path)
                .map_err(|error| format!("failed to inspect {}: {error}", path.display()))
        })
        .transpose()?;
    let output = (!check)
        .then(|| cache_path(path, options.python_version, cache_optimization))
        .transpose()?;
    let cached = output
        .as_deref()
        .filter(|_| !force)
        .and_then(read_cache_header);
    let mut expected = [0; 16];
    let mut source_mtime = 0;
    expected[..4].copy_from_slice(&options.python_version.target().magic_number);
    expected[4..8].copy_from_slice(&invalidation_mode.flags().to_le_bytes());
    if let Some(metadata) = &metadata
        && let InvalidationMode::Timestamp = invalidation_mode
    {
        let modified = metadata
            .modified()
            .map_err(|error| format!("failed to read the mtime for {}: {error}", path.display()))?;
        source_mtime = encode_mtime(modified);
        expected[8..12].copy_from_slice(&source_mtime.to_le_bytes());
        expected[12..].copy_from_slice(&low_u32(metadata.len()).to_le_bytes());
        if cached == Some(expected) {
            return Ok(false);
        }
    }
    let source_bytes = read_source(path, metadata.as_ref())
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let source_hash = (!check && invalidation_mode.flags() != 0)
        .then(|| options.python_version.target().source_hash(&source_bytes));
    if let Some(source_hash) = source_hash {
        expected[8..].copy_from_slice(&source_hash);
        if cached == Some(expected) {
            return Ok(false);
        }
    }
    let module =
        compile_bytes_with_path_and_options(&source_bytes, path, options).map_err(|error| {
            match &error {
                CompileError::Decode(error) => {
                    format!("failed to decode {}: {error}", path.display())
                }
                _ => format!("failed to compile {}: {error}", path.display()),
            }
        })?;

    let (Some(metadata), Some(output)) = (metadata, output) else {
        black_box(module);
        return Ok(true);
    };

    let pyc = if let Some(source_hash) = source_hash {
        module.to_hash_pyc_with_hash(source_hash, invalidation_mode.flags() == 3)
    } else {
        module.to_timestamp_pyc(source_mtime, low_u32(metadata.len()))
    };

    let parent = output.parent().ok_or_else(|| {
        format!(
            "failed to determine the cache directory for {}",
            path.display()
        )
    })?;
    let mut temporary = if let Ok(temporary) = temporary_cache_file(parent, &metadata) {
        temporary
    } else {
        fs_err::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        temporary_cache_file(parent, &metadata)
            .map_err(|error| format!("failed to create a temporary cache file: {error}"))?
    };
    temporary
        .write_all(&pyc)
        .map_err(|error| format!("failed to write a temporary cache file: {error}"))?;
    temporary
        .persist(&output)
        .map(|_| true)
        .map_err(|error| format!("failed to replace {}: {error}", output.display()))
}

/// Treat missing, unreadable, truncated, and nonregular cache destinations as misses.
fn read_cache_header(output: &Path) -> Option<[u8; 16]> {
    // Never follow an output symlink. Compilation replaces it atomically instead.
    if !fs_err::symlink_metadata(output).ok()?.is_file() {
        return None;
    }
    let mut header = [0; 16];
    fs_err::File::open(output)
        .ok()?
        .read_exact(&mut header)
        .ok()?;
    Some(header)
}

/// Reuse the timestamp snapshot as a size hint without querying the opened file again.
fn read_source(path: &Path, metadata: Option<&std::fs::Metadata>) -> io::Result<Vec<u8>> {
    let Some(metadata) = metadata else {
        return fs_err::read(path);
    };
    // The path can change before it is opened, and special files can report zero bytes.
    // Cap the initial allocation and still read through EOF regardless of the snapshot.
    let capacity = usize::try_from(metadata.len())
        .unwrap_or(usize::MAX)
        .min(1024 * 1024)
        + 1;
    let mut contents = vec![0; capacity];
    let mut source = fs_err::File::open(path)?;
    let mut read = 0;
    while read < contents.len() {
        match source.read(&mut contents[read..]) {
            Ok(0) => {
                contents.truncate(read);
                return Ok(contents);
            }
            Ok(count) => read += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    source.read_to_end(&mut contents)?;
    Ok(contents)
}

/// Encodes signed Unix seconds for a CPython timestamp cache header.
///
/// CPython first exposes `st_mtime` as a float, then truncates toward zero and wraps
/// modulo 2^32. Rounding can move a nanosecond timestamp into the next second.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "match CPython's floating-point stat time; the final remainder fits in u32"
)]
fn encode_mtime(modified: SystemTime) -> u32 {
    let (seconds, nanos) = match modified.duration_since(UNIX_EPOCH) {
        Ok(duration) => (i128::from(duration.as_secs()), duration.subsec_nanos()),
        Err(error) => {
            let duration = error.duration();
            let seconds = -i128::from(duration.as_secs());
            if duration.subsec_nanos() == 0 {
                (seconds, 0)
            } else {
                (seconds - 1, 1_000_000_000 - duration.subsec_nanos())
            }
        }
    };
    // Match Modules/posixmodule.c's `sec + 1e-9*nsec`, including before the epoch.
    let mtime = seconds as f64 + 1e-9 * f64::from(nanos);
    mtime.trunc().rem_euclid(4_294_967_296.0) as u32
}

fn low_u32(value: u64) -> u32 {
    let [first, second, third, fourth, _, _, _, _] = value.to_le_bytes();
    u32::from_le_bytes([first, second, third, fourth])
}

#[cfg(unix)]
fn temporary_cache_file(
    parent: &Path,
    source: &std::fs::Metadata,
) -> io::Result<tempfile::NamedTempFile> {
    use std::os::unix::fs::PermissionsExt;

    let mode = (source.permissions().mode() & 0o666) | 0o200;
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(mode))
        .tempfile_in(parent)
}

#[cfg(not(unix))]
fn temporary_cache_file(
    parent: &Path,
    _source: &std::fs::Metadata,
) -> io::Result<tempfile::NamedTempFile> {
    tempfile::NamedTempFile::new_in(parent)
}

fn cache_path(
    path: &Path,
    python_version: PythonVersion,
    cache_optimization: usize,
) -> Result<PathBuf, String> {
    let filename = path
        .file_name()
        .ok_or_else(|| format!("{} has no file name", path.display()))?;
    let separator = filename
        .as_encoded_bytes()
        .iter()
        .rposition(|&byte| byte == b'.');
    let mut filename = if separator == Some(0) {
        // CPython treats a lone leading dot as an extension separator. Give it a
        // nonempty stem so `Path::extension` can extract the suffix without losing
        // platform-specific filename bytes.
        let mut prefixed = OsString::from("_");
        prefixed.push(filename);
        Path::new(&prefixed)
            .extension()
            .expect("the prefixed filename has an extension")
            .to_os_string()
    } else {
        path.file_stem()
            .ok_or_else(|| format!("{} has no file stem", path.display()))?
            .to_os_string()
    };
    // Match CPython's `base + sep + tag`: extensionless names have no separator.
    if separator.is_some() {
        filename.push(".");
    }
    filename.push(cache_tag(python_version));
    if cache_optimization > 0 {
        filename.push(format!(".opt-{cache_optimization}"));
    }
    filename.push(".pyc");
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    Ok(parent.join("__pycache__").join(filename))
}

fn cache_tag(python_version: PythonVersion) -> String {
    let target = python_version.target();
    let (major, minor, _) = target.version;
    format!("{}-{major}{minor}", target.implementation)
}

fn write_error(message: &str) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "serc: {message}");
}

#[cfg(test)]
mod tests {
    use super::{
        InvalidationMode, cache_path, cache_tag, compilation_groups, compile_path_with_metadata,
        discover, encode_mtime, has_python_suffix, prepare_compilation,
        prioritize_compilation_groups, read_file_list,
    };
    use super::{default_jobs, jobs_for_inputs};
    use serc::{CompileOptions, PythonVersion};
    use std::num::NonZeroUsize;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn default_compilation_jobs_follow_rayon_and_available_inputs() {
        assert_eq!(default_jobs(None, None, || 8), 8);
        assert_eq!(default_jobs(Some("4"), None, || 8), 4);
        assert_eq!(default_jobs(Some("1"), Some("4"), || 8), 1);
        assert_eq!(default_jobs(Some("0"), Some("4"), || 8), 8);
        assert_eq!(default_jobs(Some("invalid"), None, || 8), 8);
        assert_eq!(default_jobs(Some("invalid"), Some("4"), || 8), 4);
        assert_eq!(default_jobs(None, Some("4"), || 8), 4);
        assert_eq!(default_jobs(None, Some("0"), || 8), 8);
        assert_eq!(default_jobs(None, None, || 1), 1);
        assert_eq!(jobs_for_inputs(None, 0), 0);
        assert_eq!(jobs_for_inputs(None, 1), 1);
        assert_eq!(jobs_for_inputs(NonZeroUsize::new(1), 4), 1);
        assert_eq!(jobs_for_inputs(NonZeroUsize::new(8), 4), 4);
    }

    #[test]
    fn cache_writing_limits_workers_to_independent_outputs() {
        let directory = tempfile::tempdir().unwrap();
        let paths = [
            directory.path().join("Alpha.py"),
            directory.path().join("alpha.py"),
            directory.path().join("other.py"),
            directory.path().join("caf\u{e9}.py"),
        ];

        let (jobs, groups) = prepare_compilation(&paths[..3], PythonVersion::Py314, 0, false, 3);
        assert_eq!(jobs, 2);
        assert_eq!(groups.unwrap().len(), 2);

        let (jobs, groups) = prepare_compilation(&paths, PythonVersion::Py314, 0, false, 4);
        assert_eq!(jobs, 1);
        assert_eq!(
            groups.unwrap()[0]
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        assert_eq!(
            prepare_compilation(&paths, PythonVersion::Py314, 0, true, 4),
            (4, None)
        );
        assert_eq!(
            prepare_compilation(&paths, PythonVersion::Py314, 0, false, 1),
            (1, None)
        );
        assert_eq!(
            prepare_compilation(
                &paths[..1],
                PythonVersion::Py314,
                0,
                false,
                jobs_for_inputs(None, 1)
            ),
            (1, None)
        );
    }

    #[test]
    fn target_cache_tag() {
        assert_eq!(cache_tag(PythonVersion::Py312), "cpython-312");
        assert_eq!(cache_tag(PythonVersion::Py313), "cpython-313");
        assert_eq!(cache_tag(PythonVersion::Py314), "cpython-314");
        assert_eq!(cache_tag(PythonVersion::Py315), "cpython-315");
    }

    #[test]
    fn parallel_compilation_groups_cover_case_and_unicode_aliases() {
        let directory = tempfile::tempdir().unwrap();
        let ascii = directory.path().join("ascii");
        let unicode = directory.path().join("unicode");
        let other = directory.path().join("other");
        for parent in [&ascii, &unicode, &other] {
            fs_err::create_dir(parent).unwrap();
        }
        let paths = [
            ascii.join("A.py"),
            ascii.join("a.py"),
            ascii.join("different.py"),
            unicode.join("K.py"),
            unicode.join("\u{212a}.py"),
            unicode.join("caf\u{e9}.py"),
            unicode.join("cafe\u{301}.py"),
            unicode.join("different.py"),
            other.join("a.py"),
        ];
        let indices = || {
            compilation_groups(&paths, PythonVersion::Py314, 0)
                .into_iter()
                .map(|group| {
                    group
                        .into_iter()
                        .map(|(index, _)| index)
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        let expected = [vec![0, 1], vec![2], vec![3, 4, 5, 6, 7], vec![8]];
        assert_eq!(indices(), expected);
        for parent in [&ascii, &unicode, &other] {
            fs_err::create_dir(parent.join("__pycache__")).unwrap();
        }
        assert_eq!(indices(), expected);
    }

    #[test]
    fn parallel_compilation_groups_cover_unknown_directory_aliases() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing");
        let paths = [
            directory.path().join("A.py"),
            missing.join("a.py"),
            directory.path().join("other.py"),
            missing.join("\u{212a}.py"),
        ];
        let indices = |paths: &[PathBuf]| {
            compilation_groups(paths, PythonVersion::Py314, 0)
                .into_iter()
                .map(|group| {
                    group
                        .into_iter()
                        .map(|(index, _)| index)
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(indices(&paths[..3]), [vec![0, 1], vec![2]]);
        assert_eq!(indices(&paths), [vec![0, 1, 2, 3]]);
    }

    #[test]
    fn parallel_compilation_prioritizes_total_source_size_without_reordering_aliases() {
        let directory = tempfile::tempdir().unwrap();
        let paths: Vec<_> = (0..6)
            .map(|index| directory.path().join(format!("{index}.py")))
            .collect();
        for (path, size) in paths.iter().zip([1, 4, 12, 9, 6]) {
            fs_err::write(path, vec![b' '; size]).unwrap();
        }
        let groups = vec![
            vec![(0, paths[0].as_path()), (3, paths[3].as_path())],
            vec![(1, paths[1].as_path()), (4, paths[4].as_path())],
            vec![(2, paths[2].as_path())],
            vec![(5, paths[5].as_path())],
        ];
        let indices = |groups: Vec<Vec<(usize, &Path)>>| {
            groups
                .into_iter()
                .map(|group| {
                    group
                        .into_iter()
                        .map(|(index, _)| index)
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            indices(prioritize_compilation_groups(groups.clone(), 4, None)),
            [vec![0, 3], vec![1, 4], vec![2], vec![5]]
        );
        assert_eq!(
            indices(prioritize_compilation_groups(groups.clone(), 2, None)),
            [vec![2], vec![0, 3], vec![1, 4], vec![5]]
        );
        assert_eq!(
            indices(prioritize_compilation_groups(
                groups,
                2,
                Some(&[1, 4, 12, 9, 6, 20]),
            )),
            [vec![5], vec![2], vec![0, 3], vec![1, 4]]
        );
    }

    #[test]
    fn file_lists_keep_source_sizes_aligned_after_sorting_and_deduplication() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.py");
        let second = directory.path().join("second.py");
        fs_err::write(&first, b"x = 1\n").unwrap();
        fs_err::write(&second, b"x = 123\n").unwrap();
        let manifest = directory.path().join("files.txt");
        fs_err::write(
            &manifest,
            format!(
                "{}\n{}\n{}\n",
                second.display(),
                first.display(),
                second.display()
            ),
        )
        .unwrap();
        assert_eq!(
            read_file_list(&manifest, false).unwrap(),
            (vec![first, second], vec![6, 8])
        );
    }

    #[test]
    fn builds_cache_path() {
        assert_eq!(
            cache_path(Path::new("package/module.py"), PythonVersion::Py314, 0).unwrap(),
            PathBuf::from("package/__pycache__/module.cpython-314.pyc")
        );
        assert_eq!(
            cache_path(Path::new("package/.py"), PythonVersion::Py314, 0).unwrap(),
            PathBuf::from("package/__pycache__/py.cpython-314.pyc")
        );
        assert_eq!(
            cache_path(Path::new("package/module.py"), PythonVersion::Py315, 0).unwrap(),
            PathBuf::from("package/__pycache__/module.cpython-315.pyc")
        );
        assert_eq!(
            cache_path(Path::new("package/module"), PythonVersion::Py315, 0).unwrap(),
            PathBuf::from("package/__pycache__/modulecpython-315.pyc")
        );
        for (version, tag) in [
            (PythonVersion::Py312, "cpython-312"),
            (PythonVersion::Py313, "cpython-313"),
        ] {
            assert_eq!(
                cache_path(Path::new("package/module.py"), version, 0).unwrap(),
                PathBuf::from(format!("package/__pycache__/module.{tag}.pyc")),
            );
            assert_eq!(
                cache_path(Path::new("package/module"), version, 0).unwrap(),
                PathBuf::from(format!("package/__pycache__/module{tag}.pyc")),
            );
        }
        assert!(has_python_suffix(Path::new("package/.py")));
    }

    #[cfg(unix)]
    #[test]
    fn cache_paths_preserve_non_utf8_names() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        for (name, expected) in [
            (b"\xff.py".as_slice(), b"\xff.cpython-314.pyc".as_slice()),
            (b".\xff", b"\xff.cpython-314.pyc"),
            (b"\xff", b"\xffcpython-314.pyc"),
        ] {
            assert_eq!(
                cache_path(Path::new(OsStr::from_bytes(name)), PythonVersion::Py314, 0).unwrap(),
                Path::new("__pycache__").join(OsStr::from_bytes(expected))
            );
        }
    }

    #[test]
    fn wraps_mtime_to_cpython_uint32() {
        assert_eq!(
            encode_mtime(UNIX_EPOCH - Duration::from_millis(1500)),
            u32::MAX
        );
        assert_eq!(encode_mtime(UNIX_EPOCH - Duration::from_millis(500)), 0);
        assert_eq!(
            encode_mtime(UNIX_EPOCH + Duration::from_secs(u64::from(u32::MAX) + 1)),
            0
        );
        assert_eq!(
            encode_mtime(UNIX_EPOCH + Duration::new(1_700_000_000, 999_999_999)),
            1_700_000_001
        );
        assert_eq!(
            encode_mtime(UNIX_EPOCH - Duration::new(1_700_000_000, 999_999_999)),
            0_u32.wrapping_sub(1_700_000_001)
        );
        assert_eq!(
            encode_mtime(UNIX_EPOCH + Duration::new(u64::from(u32::MAX), 999_999_999)),
            0
        );
        assert_eq!(
            encode_mtime(UNIX_EPOCH - Duration::new(u64::from(u32::MAX), 999_999_999)),
            0
        );
    }

    #[test]
    fn samples_source_metadata_before_reading() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("module.py");
        let source = "value = 1\n";
        let replacement = "value = 2\n";
        let replacement_mtime = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        fs_err::write(&source_path, source).unwrap();

        compile_path_with_metadata(
            &source_path,
            false,
            false,
            InvalidationMode::Timestamp,
            &CompileOptions::default(),
            0,
            |path| {
                fs_err::write(path, replacement)?;
                let file = std::fs::OpenOptions::new().write(true).open(path)?;
                file.set_times(std::fs::FileTimes::new().set_modified(replacement_mtime))?;
                fs_err::metadata(path)
            },
        )
        .unwrap();

        let expected = serc::compile_bytes(replacement.as_bytes(), source_path.to_str().unwrap())
            .unwrap()
            .to_timestamp_pyc(1_700_000_000, u32::try_from(replacement.len()).unwrap());
        assert_eq!(
            fs_err::read(cache_path(&source_path, PythonVersion::Py314, 0).unwrap()).unwrap(),
            expected
        );
    }

    #[test]
    fn current_timestamp_cache_does_not_read_source() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("module.py");
        fs_err::write(&source, "pass\n").unwrap();
        let options = CompileOptions::default();
        compile_path_with_metadata(
            &source,
            false,
            false,
            InvalidationMode::Timestamp,
            &options,
            0,
            |path| fs_err::metadata(path),
        )
        .unwrap();
        let compiled = compile_path_with_metadata(
            &source,
            false,
            false,
            InvalidationMode::Timestamp,
            &options,
            0,
            |path| {
                let metadata = fs_err::metadata(path)?;
                fs_err::remove_file(path)?;
                Ok(metadata)
            },
        )
        .unwrap();
        assert!(!compiled);
    }

    #[test]
    fn reads_replaced_sources_through_eof_with_the_original_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("module.py");
        let original_mtime = UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        for (original, replacement) in [("", "value = 123456\n"), ("value = 123456\n", "")] {
            fs_err::write(&source_path, original).unwrap();
            let source = fs_err::File::options()
                .write(true)
                .open(&source_path)
                .unwrap();
            source
                .set_times(std::fs::FileTimes::new().set_modified(original_mtime))
                .unwrap();
            drop(source);
            compile_path_with_metadata(
                &source_path,
                false,
                false,
                InvalidationMode::Timestamp,
                &CompileOptions::default(),
                0,
                |path| {
                    let metadata = fs_err::metadata(path)?;
                    let replacement_path = directory.path().join("replacement.py");
                    fs_err::write(&replacement_path, replacement)?;
                    fs_err::rename(&replacement_path, path)?;
                    Ok(metadata)
                },
            )
            .unwrap();

            let expected =
                serc::compile_bytes(replacement.as_bytes(), source_path.to_str().unwrap())
                    .unwrap()
                    .to_timestamp_pyc(1_700_000_000, u32::try_from(original.len()).unwrap());
            assert_eq!(
                fs_err::read(cache_path(&source_path, PythonVersion::Py314, 0).unwrap()).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn reports_cache_directory_creation_errors() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("module.py");
        let cache_directory = directory.path().join("__pycache__");
        fs_err::write(&source_path, "pass\n").unwrap();
        fs_err::write(&cache_directory, "not a directory").unwrap();

        let error = compile_path_with_metadata(
            &source_path,
            false,
            false,
            InvalidationMode::Timestamp,
            &CompileOptions::default(),
            0,
            |path| fs_err::metadata(path),
        )
        .unwrap_err();
        assert!(error.starts_with(&format!("failed to create {}:", cache_directory.display())));
    }

    #[test]
    fn discovers_python_files_in_stable_order() {
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("nested");
        fs_err::create_dir_all(&nested).unwrap();
        fs_err::write(directory.path().join("b.py"), "pass\n").unwrap();
        fs_err::write(directory.path().join("ignored.txt"), "pass\n").unwrap();
        fs_err::write(nested.join("a.py"), "pass\n").unwrap();

        let paths = discover(&[directory.path().to_path_buf()]).unwrap();

        assert_eq!(
            paths,
            vec![directory.path().join("b.py"), nested.join("a.py")]
        );
    }

    #[test]
    fn discovery_prunes_python_cache_directories() {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("__pycache__");
        let source = directory.path().join("module.py");
        fs_err::create_dir(&cache).unwrap();
        fs_err::write(&source, "pass\n").unwrap();
        fs_err::write(cache.join("stray.py"), "pass\n").unwrap();

        assert_eq!(
            discover(&[directory.path().to_path_buf()]).unwrap(),
            [source]
        );
    }

    #[cfg(unix)]
    #[test]
    fn discovers_symlinked_python_files() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let source = directory.path().join("module.py");
        fs_err::write(&target, "pass\n").unwrap();
        symlink(&target, &source).unwrap();

        assert_eq!(
            discover(&[directory.path().to_path_buf()]).unwrap(),
            [source]
        );
    }
}
