use std::collections::HashMap;
use std::ffi::OsString;
use std::hint::black_box;
use std::io::{self, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::builder::Styles;
use clap::builder::styling::{AnsiColor, Effects};
use clap::{Parser, Subcommand, ValueHint};
use serc::{CompileError, CompileOptions, PythonVersion, compile_bytes_with_options};
use rayon::prelude::*;
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
    /// Compile Python files to timestamp-invalidated .pyc files.
    Compile(CompileCommand),
}

#[derive(Debug, clap::Args)]
struct CompileCommand {
    /// Python files or directories to compile [default: .]
    #[arg(value_hint = ValueHint::AnyPath)]
    paths: Vec<PathBuf>,

    /// Python language and bytecode version to target (3.12, 3.13, 3.14, or 3.15).
    #[arg(long, default_value_t = PythonVersion::default())]
    python_version: PythonVersion,

    /// Maximum number of files to compile in parallel [default: available CPUs].
    #[arg(short, long)]
    jobs: Option<NonZeroUsize>,

    /// Compile the sources without writing .pyc files.
    #[arg(long)]
    check: bool,

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
    let roots = if command.paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        command.paths
    };
    let paths = discover(&roots)?;
    let mut failures = 0_usize;
    let options = CompileOptions {
        python_version: command.python_version,
        ..CompileOptions::default()
    };

    let jobs = jobs_for_inputs(command.jobs, paths.len());
    if jobs <= 1 {
        for path in &paths {
            if let Err(message) = compile_path(path, command.check, &options) {
                failures += 1;
                write_error(&message);
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
            if command.check {
                paths
                    .par_iter()
                    .enumerate()
                    .filter_map(|(index, path)| {
                        compile_path(path, true, &options)
                            .err()
                            .map(|message| (index, message))
                    })
                    .collect()
            } else {
                compilation_groups(&paths, options.python_version)
                    .into_par_iter()
                    .flat_map_iter(|group| {
                        group.into_iter().filter_map(|(index, path)| {
                            compile_path(path, false, &options)
                                .err()
                                .map(|message| (index, message))
                        })
                    })
                    .collect()
            }
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
        let noun = if paths.len() == 1 { "file" } else { "files" };
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "Compiled {} {noun}", paths.len())
            .map_err(|error| format!("failed to write to stdout: {error}"))?;
    }

    Ok(ExitCode::SUCCESS)
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

/// Keep inputs that share an output in discovery order so the same cache wins as in serial mode.
fn compilation_groups(
    paths: &[PathBuf],
    python_version: PythonVersion,
) -> Vec<Vec<(usize, &Path)>> {
    use std::collections::HashSet;
    use std::sync::Arc;

    let mut directories = HashMap::new();
    let mut non_ascii_directories = HashSet::new();
    let mut unknown_directory = false;
    let mut identities = Vec::with_capacity(paths.len());
    for path in paths {
        let output = cache_path(path, python_version).ok().and_then(|output| {
            let parent = output.parent()?;
            let filename = output.file_name()?;
            let directory = Arc::clone(
                directories
                    .entry(parent.to_owned())
                    .or_insert_with(|| Arc::new(cache_directory_identity(parent))),
            );
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

fn compile_path(path: &Path, check: bool, options: &CompileOptions) -> Result<(), String> {
    compile_path_with_metadata(path, check, options, |path| fs_err::metadata(path))
}

/// Compiles a source file, optionally replacing its timestamp-invalidated cache.
///
/// Check mode skips both the metadata loader and cache output.
fn compile_path_with_metadata(
    path: &Path,
    check: bool,
    options: &CompileOptions,
    load_metadata: impl FnOnce(&Path) -> io::Result<std::fs::Metadata>,
) -> Result<(), String> {
    // Snapshot timestamp-cache metadata before reading. If the path changes during compilation,
    // the resulting header describes the old state and cannot validate the new source as fresh.
    let metadata = (!check)
        .then(|| {
            load_metadata(path)
                .map_err(|error| format!("failed to inspect {}: {error}", path.display()))
        })
        .transpose()?;
    let source_bytes = fs_err::read(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let filename = path
        .to_str()
        .ok_or_else(|| format!("path is not UTF-8: {}", path.display()))?;
    let module =
        compile_bytes_with_options(&source_bytes, filename, options).map_err(
            |error| match &error {
                CompileError::Decode(error) => {
                    format!("failed to decode {}: {error}", path.display())
                }
                _ => format!("failed to compile {}: {error}", path.display()),
            },
        )?;

    let Some(metadata) = metadata else {
        black_box(module);
        return Ok(());
    };

    let source_size = metadata.len();
    let modified = metadata
        .modified()
        .map_err(|error| format!("failed to read the mtime for {}: {error}", path.display()))?;
    let source_mtime = encode_mtime(modified);
    let pyc = module.to_timestamp_pyc(source_mtime, low_u32(source_size));
    let output = cache_path(path, options.python_version)?;

    let parent = output.parent().ok_or_else(|| {
        format!(
            "failed to determine the cache directory for {}",
            path.display()
        )
    })?;
    fs_err::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    let mut temporary = temporary_cache_file(parent, &metadata)
        .map_err(|error| format!("failed to create a temporary cache file: {error}"))?;
    temporary
        .write_all(&pyc)
        .map_err(|error| format!("failed to write a temporary cache file: {error}"))?;
    temporary
        .persist(&output)
        .map(|_| ())
        .map_err(|error| format!("failed to replace {}: {error}", output.display()))
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

fn cache_path(path: &Path, python_version: PythonVersion) -> Result<PathBuf, String> {
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
        cache_path, cache_tag, compilation_groups, compile_path_with_metadata, discover,
        encode_mtime, has_python_suffix,
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
        assert_eq!(default_jobs(None, Some("4"), || 8), 4);
        assert_eq!(default_jobs(None, Some("0"), || 8), 8);
        assert_eq!(default_jobs(None, None, || 1), 1);
        assert_eq!(jobs_for_inputs(None, 0), 0);
        assert_eq!(jobs_for_inputs(None, 1), 1);
        assert_eq!(jobs_for_inputs(NonZeroUsize::new(1), 4), 1);
        assert_eq!(jobs_for_inputs(NonZeroUsize::new(8), 4), 4);
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
            compilation_groups(&paths, PythonVersion::Py314)
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
            compilation_groups(paths, PythonVersion::Py314)
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
    fn builds_cache_path() {
        assert_eq!(
            cache_path(Path::new("package/module.py"), PythonVersion::Py314).unwrap(),
            PathBuf::from("package/__pycache__/module.cpython-314.pyc")
        );
        assert_eq!(
            cache_path(Path::new("package/.py"), PythonVersion::Py314).unwrap(),
            PathBuf::from("package/__pycache__/py.cpython-314.pyc")
        );
        assert_eq!(
            cache_path(Path::new("package/module.py"), PythonVersion::Py315).unwrap(),
            PathBuf::from("package/__pycache__/module.cpython-315.pyc")
        );
        assert_eq!(
            cache_path(Path::new("package/module"), PythonVersion::Py315).unwrap(),
            PathBuf::from("package/__pycache__/modulecpython-315.pyc")
        );
        for (version, tag) in [
            (PythonVersion::Py312, "cpython-312"),
            (PythonVersion::Py313, "cpython-313"),
        ] {
            assert_eq!(
                cache_path(Path::new("package/module.py"), version).unwrap(),
                PathBuf::from(format!("package/__pycache__/module.{tag}.pyc")),
            );
            assert_eq!(
                cache_path(Path::new("package/module"), version).unwrap(),
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
                cache_path(Path::new(OsStr::from_bytes(name)), PythonVersion::Py314).unwrap(),
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

        compile_path_with_metadata(&source_path, false, &CompileOptions::default(), |path| {
            fs_err::write(path, replacement)?;
            let file = std::fs::OpenOptions::new().write(true).open(path)?;
            file.set_times(std::fs::FileTimes::new().set_modified(replacement_mtime))?;
            fs_err::metadata(path)
        })
        .unwrap();

        let expected =
            serc::compile_bytes(replacement.as_bytes(), source_path.to_str().unwrap())
                .unwrap()
                .to_timestamp_pyc(1_700_000_000, u32::try_from(replacement.len()).unwrap());
        assert_eq!(
            fs_err::read(cache_path(&source_path, PythonVersion::Py314).unwrap()).unwrap(),
            expected
        );
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
