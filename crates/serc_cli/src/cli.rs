use std::ffi::OsString;
use std::hint::black_box;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::builder::Styles;
use clap::builder::styling::{AnsiColor, Effects};
use clap::{Parser, Subcommand, ValueHint};
use serc::{CPYTHON_TARGET, CompileError, compile_bytes};
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

    for path in &paths {
        if let Err(message) = compile_path(path, command.check) {
            failures += 1;
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

fn compile_path(path: &Path, check: bool) -> Result<(), String> {
    compile_path_with_metadata(path, check, |path| fs_err::metadata(path))
}

/// Compiles a source file, optionally replacing its timestamp-invalidated cache.
///
/// Check mode skips both the metadata loader and cache output.
fn compile_path_with_metadata(
    path: &Path,
    check: bool,
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
    let module = compile_bytes(&source_bytes, filename).map_err(|error| match &error {
        CompileError::Decode(error) => {
            format!("failed to decode {}: {error}", path.display())
        }
        _ => format!("failed to compile {}: {error}", path.display()),
    })?;

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
    let output = cache_path(path)?;

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

fn cache_path(path: &Path) -> Result<PathBuf, String> {
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
    filename.push(cache_tag());
    filename.push(".pyc");
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    Ok(parent.join("__pycache__").join(filename))
}

fn cache_tag() -> String {
    let (major, minor, _) = CPYTHON_TARGET.version;
    format!("{}-{major}{minor}", CPYTHON_TARGET.implementation)
}

fn write_error(message: &str) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "serc: {message}");
}

#[cfg(test)]
mod tests {
    use super::{
        cache_path, cache_tag, compile_path_with_metadata, discover, encode_mtime,
        has_python_suffix,
    };
    use std::path::{Path, PathBuf};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn target_cache_tag() {
        assert_eq!(cache_tag(), "cpython-314");
    }

    #[test]
    fn builds_cache_path() {
        assert_eq!(
            cache_path(Path::new("package/module.py")).unwrap(),
            PathBuf::from("package/__pycache__/module.cpython-314.pyc")
        );
        assert_eq!(
            cache_path(Path::new("package/.py")).unwrap(),
            PathBuf::from("package/__pycache__/py.cpython-314.pyc")
        );
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
                cache_path(Path::new(OsStr::from_bytes(name))).unwrap(),
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

        compile_path_with_metadata(&source_path, false, |path| {
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
            fs_err::read(cache_path(&source_path).unwrap()).unwrap(),
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
