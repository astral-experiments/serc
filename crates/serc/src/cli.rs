use std::ffi::OsString;
use std::hint::black_box;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::UNIX_EPOCH;

use clap::builder::Styles;
use clap::builder::styling::{AnsiColor, Effects};
use clap::{Parser, Subcommand, ValueHint};
use serc::{CPYTHON_TARGET, compile, compile_to_pyc};
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

        for entry in WalkDir::new(root).follow_links(false) {
            let entry =
                entry.map_err(|error| format!("failed to walk {}: {error}", root.display()))?;
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
    if paths.is_empty() {
        return Err("no Python files found".to_string());
    }
    Ok(paths)
}

fn compile_path(path: &Path, check: bool) -> Result<(), String> {
    let source = fs_err::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let filename = path
        .to_str()
        .ok_or_else(|| format!("path is not UTF-8: {}", path.display()))?;

    if check {
        let module = compile(&source, filename)
            .map_err(|error| format!("failed to compile {}: {error}", path.display()))?;
        black_box(module);
        return Ok(());
    }

    let metadata = fs_err::metadata(path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
    let modified = metadata
        .modified()
        .map_err(|error| format!("failed to read the mtime for {}: {error}", path.display()))?;
    let seconds = modified.duration_since(UNIX_EPOCH).map_err(|error| {
        format!(
            "{} has a modification time before the Unix epoch: {error}",
            path.display()
        )
    })?;
    let source_mtime = u32::try_from(seconds.as_secs())
        .map_err(|error| format!("failed to encode the mtime for {}: {error}", path.display()))?;
    let pyc = compile_to_pyc(&source, filename, source_mtime)
        .map_err(|error| format!("failed to compile {}: {error}", path.display()))?;
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
    let stem = path
        .file_stem()
        .ok_or_else(|| format!("{} has no file name", path.display()))?;
    let mut filename = OsString::from(stem);
    filename.push(format!(".{}.pyc", cache_tag()));
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
    use super::{cache_path, cache_tag, discover};
    use std::path::{Path, PathBuf};

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
}
