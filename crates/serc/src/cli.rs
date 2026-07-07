use std::ffi::OsString;
use std::hint::black_box;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::builder::Styles;
use clap::builder::styling::{AnsiColor, Effects};
use clap::{Parser, Subcommand, ValueHint};
use serc::{CPYTHON_TARGET, compile};
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
    compile_path_with_metadata(path, check, source_metadata)
}

fn source_metadata(path: &Path) -> io::Result<std::fs::Metadata> {
    fs_err::metadata(path)
}

fn compile_path_with_metadata(
    path: &Path,
    check: bool,
    load_metadata: impl FnOnce(&Path) -> io::Result<std::fs::Metadata>,
) -> Result<(), String> {
    let source_bytes = fs_err::read(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let source = decode_source(&source_bytes)
        .map_err(|error| format!("failed to decode {}: {error}", path.display()))?;
    let filename = path
        .to_str()
        .ok_or_else(|| format!("path is not UTF-8: {}", path.display()))?;
    let module = compile(&source, filename)
        .map_err(|error| format!("failed to compile {}: {error}", path.display()))?;

    if check {
        black_box(module);
        return Ok(());
    }

    let metadata = load_metadata(path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
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

enum SourceEncoding {
    Utf8,
    Latin1,
    Ascii,
    Iso8859_9,
    Cp437,
    Standard(&'static encoding_rs::Encoding),
}

fn decode_source(bytes: &[u8]) -> Result<String, String> {
    const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

    let (bytes, has_bom) = bytes
        .strip_prefix(UTF8_BOM)
        .map_or((bytes, false), |bytes| (bytes, true));
    let cookie = declared_encoding(bytes);
    let encoding = if let Some(cookie) = cookie {
        let Some((encoding, bom_compatible)) = classify_encoding(cookie) else {
            return Err(format!("unsupported source encoding `{cookie}`"));
        };
        if has_bom && !bom_compatible {
            return Err(format!(
                "encoding problem: UTF-8 BOM conflicts with `{cookie}`"
            ));
        }
        encoding
    } else {
        SourceEncoding::Utf8
    };

    match encoding {
        SourceEncoding::Utf8 => std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|error| format!("source is not valid UTF-8: {error}")),
        SourceEncoding::Latin1 => Ok(bytes.iter().copied().map(char::from).collect()),
        SourceEncoding::Ascii => {
            if !bytes.is_ascii() {
                return Err("source is not valid ASCII".to_string());
            }
            std::str::from_utf8(bytes)
                .map(str::to_owned)
                .map_err(|error| format!("source is not valid ASCII: {error}"))
        }
        SourceEncoding::Iso8859_9 => Ok(bytes
            .iter()
            .copied()
            .map(|byte| match byte {
                0xd0 => '\u{011e}',
                0xdd => '\u{0130}',
                0xde => '\u{015e}',
                0xf0 => '\u{011f}',
                0xfd => '\u{0131}',
                0xfe => '\u{015f}',
                _ => char::from(byte),
            })
            .collect()),
        SourceEncoding::Cp437 => Ok(bytes
            .iter()
            .copied()
            .map(|byte| {
                if byte.is_ascii() {
                    char::from(byte)
                } else {
                    CP437_HIGH[usize::from(byte - 0x80)]
                }
            })
            .collect()),
        SourceEncoding::Standard(encoding) => encoding
            .decode_without_bom_handling_and_without_replacement(bytes)
            .map(std::borrow::Cow::into_owned)
            .ok_or_else(|| format!("source is not valid {}", encoding.name())),
    }
}

fn declared_encoding(bytes: &[u8]) -> Option<&str> {
    let first_end = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |index| index + 1);
    let first = &bytes[..first_end];
    if let Some(cookie) = encoding_cookie(first) {
        return Some(cookie);
    }
    if !is_blank_or_comment(first) || first_end == bytes.len() {
        return None;
    }

    let remaining = &bytes[first_end..];
    let second_end = remaining
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(remaining.len(), |index| index + 1);
    encoding_cookie(&remaining[..second_end])
}

fn encoding_cookie(line: &[u8]) -> Option<&str> {
    let comment_start = line
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | 0x0c))?;
    if line[comment_start] != b'#' {
        return None;
    }
    let comment = &line[comment_start + 1..];

    for index in 0..comment.len().saturating_sub(6) {
        if &comment[index..index + 6] != b"coding" || !matches!(comment[index + 6], b':' | b'=') {
            continue;
        }
        let mut start = index + 7;
        while matches!(comment.get(start), Some(b' ' | b'\t')) {
            start += 1;
        }
        let mut end = start;
        while comment
            .get(end)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            end += 1;
        }
        if start < end {
            return std::str::from_utf8(&comment[start..end]).ok();
        }
    }
    None
}

fn is_blank_or_comment(line: &[u8]) -> bool {
    line.iter()
        .find(|byte| !matches!(byte, b' ' | b'\t' | 0x0c))
        .is_none_or(|byte| matches!(byte, b'#' | b'\r' | b'\n'))
}

fn classify_encoding(name: &str) -> Option<(SourceEncoding, bool)> {
    let normalized_prefix: String = name
        .bytes()
        .take(12)
        .map(|byte| match byte {
            b'A'..=b'Z' => char::from(byte.to_ascii_lowercase()),
            b'_' => '-',
            _ => char::from(byte),
        })
        .collect();
    if normalized_prefix == "utf-8" || normalized_prefix.starts_with("utf-8-") {
        return Some((SourceEncoding::Utf8, true));
    }
    if matches!(
        normalized_prefix.as_str(),
        "latin-1" | "iso-8859-1" | "iso-latin-1"
    ) || normalized_prefix.starts_with("latin-1-")
        || normalized_prefix.starts_with("iso-8859-1-")
        || normalized_prefix.starts_with("iso-latin-1-")
    {
        return Some((SourceEncoding::Latin1, false));
    }

    let codec_name = normalize_codec_name(name);
    let encoding = match codec_name.as_str() {
        "utf" | "u8" | "utf8" | "utf8_ucs2" | "utf8_ucs4" | "cp65001" => SourceEncoding::Utf8,
        "latin" | "latin1" | "l1" | "8859" | "cp819" | "ibm819" | "csisolatin1" | "iso8859"
        | "iso8859_1" | "iso_8859_1_1987" | "iso_ir_100" => SourceEncoding::Latin1,
        "ascii" | "646" | "cp367" | "ibm367" | "csascii" | "us" | "us_ascii" | "iso646_us"
        | "iso_646.irv_1991" | "iso_ir_6" | "ansi_x3.4_1968" | "ansi_x3_4_1968"
        | "ansi_x3.4_1986" => SourceEncoding::Ascii,
        "iso8859_9" | "iso_8859_9" | "iso_8859_9_1989" | "iso_ir_148" | "latin5" | "l5"
        | "csisolatin5" => SourceEncoding::Iso8859_9,
        "cp437" | "437" | "ibm437" | "cspc8codepage437" => SourceEncoding::Cp437,
        "cp1252" | "1252" | "windows_1252" => SourceEncoding::Standard(encoding_rs::WINDOWS_1252),
        _ => {
            let label = codec_name.replace('_', "-");
            SourceEncoding::Standard(encoding_rs::Encoding::for_label_no_replacement(
                label.as_bytes(),
            )?)
        }
    };
    Some((encoding, false))
}

fn normalize_codec_name(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    let mut separator = false;
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'.' {
            if separator && !normalized.is_empty() {
                normalized.push('_');
            }
            normalized.push(char::from(byte.to_ascii_lowercase()));
            separator = false;
        } else {
            separator = true;
        }
    }
    normalized
}

const CP437_HIGH: [char; 128] = [
    '\u{00c7}', '\u{00fc}', '\u{00e9}', '\u{00e2}', '\u{00e4}', '\u{00e0}', '\u{00e5}', '\u{00e7}',
    '\u{00ea}', '\u{00eb}', '\u{00e8}', '\u{00ef}', '\u{00ee}', '\u{00ec}', '\u{00c4}', '\u{00c5}',
    '\u{00c9}', '\u{00e6}', '\u{00c6}', '\u{00f4}', '\u{00f6}', '\u{00f2}', '\u{00fb}', '\u{00f9}',
    '\u{00ff}', '\u{00d6}', '\u{00dc}', '\u{00a2}', '\u{00a3}', '\u{00a5}', '\u{20a7}', '\u{0192}',
    '\u{00e1}', '\u{00ed}', '\u{00f3}', '\u{00fa}', '\u{00f1}', '\u{00d1}', '\u{00aa}', '\u{00ba}',
    '\u{00bf}', '\u{2310}', '\u{00ac}', '\u{00bd}', '\u{00bc}', '\u{00a1}', '\u{00ab}', '\u{00bb}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{2561}', '\u{2562}', '\u{2556}',
    '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255d}', '\u{255c}', '\u{255b}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252c}', '\u{251c}', '\u{2500}', '\u{253c}', '\u{255e}', '\u{255f}',
    '\u{255a}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256c}', '\u{2567}',
    '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}', '\u{2552}', '\u{2553}', '\u{256b}',
    '\u{256a}', '\u{2518}', '\u{250c}', '\u{2588}', '\u{2584}', '\u{258c}', '\u{2590}', '\u{2580}',
    '\u{03b1}', '\u{00df}', '\u{0393}', '\u{03c0}', '\u{03a3}', '\u{03c3}', '\u{00b5}', '\u{03c4}',
    '\u{03a6}', '\u{0398}', '\u{03a9}', '\u{03b4}', '\u{221e}', '\u{03c6}', '\u{03b5}', '\u{2229}',
    '\u{2261}', '\u{00b1}', '\u{2265}', '\u{2264}', '\u{2320}', '\u{2321}', '\u{00f7}', '\u{2248}',
    '\u{00b0}', '\u{2219}', '\u{00b7}', '\u{221a}', '\u{207f}', '\u{00b2}', '\u{25a0}', '\u{00a0}',
];

fn encode_mtime(modified: SystemTime) -> u32 {
    match modified.duration_since(UNIX_EPOCH) {
        Ok(duration) => low_u32(duration.as_secs()),
        Err(error) => 0_u32.wrapping_sub(low_u32(error.duration().as_secs())),
    }
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
    let stem = if filename == ".py" {
        std::ffi::OsStr::new("py")
    } else {
        path.file_stem()
            .ok_or_else(|| format!("{} has no file stem", path.display()))?
    };
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
    use super::{
        cache_path, cache_tag, compile_path_with_metadata, decode_source, discover, encode_mtime,
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
    }

    #[test]
    fn decodes_pep_263_encoding_cookies() {
        assert_eq!(
            decode_source(b"# comment\n# coding: latin-1\nvalue = '\xe9'\n").unwrap(),
            "# comment\n# coding: latin-1\nvalue = '\u{e9}'\n"
        );
        assert_eq!(
            decode_source(b"# coding: cp1252\nvalue = '\x80'\n").unwrap(),
            "# coding: cp1252\nvalue = '\u{20ac}'\n"
        );
        assert!(decode_source(b"# coding: ascii\nvalue = 1\n").is_ok());
        assert_eq!(
            decode_source(b"# coding: ascii\nvalue = '\x80'\n").unwrap_err(),
            "source is not valid ASCII"
        );
    }

    #[test]
    fn samples_source_metadata_after_compilation() {
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

        let expected = serc::compile(source, source_path.to_str().unwrap())
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
