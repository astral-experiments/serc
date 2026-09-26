use std::path::Path;
use std::process::Command;

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options};

/// Generate `py_compile` output only after checking the exact oracle's release and magic.
fn compile_with_cpython(version: PythonVersion, directory: &Path) {
    let (variable, default, release_level, release_serial) = match version {
        PythonVersion::Py312 => ("SERC_TEST_PYTHON_312", "python3.12", "final", "0"),
        PythonVersion::Py313 => ("SERC_TEST_PYTHON_313", "python3.13", "final", "0"),
        PythonVersion::Py314 => ("SERC_TEST_PYTHON", "python3.14", "final", "0"),
        PythonVersion::Py315 => ("SERC_TEST_PYTHON_315", "python3.15", "candidate", "2"),
    };
    let executable = std::env::var(variable).unwrap_or_else(|_| default.to_string());
    let target = version.target();
    let output = Command::new(&executable)
        .args([
            "-I",
            "-S",
            "-c",
            r"
import importlib.util
import pathlib
import py_compile
import sys

root = pathlib.Path(sys.argv[1])
expected_version = tuple(map(int, sys.argv[2].split('.'))) + (sys.argv[5], int(sys.argv[6]))
actual_version = sys.version_info
actual_magic = importlib.util.MAGIC_NUMBER.hex()
if (sys.implementation.name != 'cpython' or actual_version != expected_version
        or sys.version.split()[0] != sys.argv[3] or actual_magic != sys.argv[4]):
    raise SystemExit(f'wrong oracle: {sys.implementation.name} {sys.version} magic {actual_magic}')
for source in sorted(root.glob('*.py')):
    for name, mode in [('checked', py_compile.PycInvalidationMode.CHECKED_HASH),
                       ('unchecked', py_compile.PycInvalidationMode.UNCHECKED_HASH)]:
        py_compile.compile(str(source), cfile=str(source) + '.' + name,
                           dfile='module.py', doraise=True, optimize=0, invalidation_mode=mode)
",
        ])
        .arg(directory)
        .arg(target.version_string())
        .arg(target.tag.strip_prefix('v').unwrap())
        .arg(target.magic_hex())
        .arg(release_level)
        .arg(release_serial)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {variable} ({executable}): {error}"));
    assert!(
        output.status.success(),
        "{variable} must identify the pinned CPython {} with magic {}: {}",
        target.tag,
        target.magic_hex(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn hash_pyc_matches_cpython_for_raw_source_and_all_targets() {
    let mut sources: Vec<Vec<u8>> = (0..=33).map(|length| vec![b'#'; length]).collect();
    // Cover the length byte wrapping in SipHash's final block as well as every short tail.
    sources.extend([255, 256, 257, 511, 512, 513].map(|length| vec![b'#'; length]));
    sources.extend([
        b"answer = 42\n".to_vec(),
        b"\xef\xbb\xbf# coding: utf-8\r\nvalue = 'caf\xc3\xa9'\r\n".to_vec(),
        b"# coding: latin-1\r\nvalue = '\xe9'\r\n".to_vec(),
    ]);

    for python_version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        let directory = tempfile::tempdir().unwrap();
        for (index, source) in sources.iter().enumerate() {
            fs_err::write(directory.path().join(format!("{index}.py")), source).unwrap();
        }
        compile_with_cpython(python_version, directory.path());

        let options = CompileOptions {
            python_version,
            ..CompileOptions::default()
        };
        for (index, source) in sources.iter().enumerate() {
            let module = compile_bytes_with_options(source, "module.py", &options).unwrap();
            let checked = module.to_hash_pyc(source, true);
            let unchecked = module.to_hash_pyc(source, false);
            let timestamp = module.to_timestamp_pyc(123, u32::try_from(source.len()).unwrap());

            assert_eq!(&checked[..4], &python_version.target().magic_number);
            assert_eq!(&checked[4..8], &3_u32.to_le_bytes());
            assert_eq!(&unchecked[4..8], &1_u32.to_le_bytes());
            assert_eq!(&checked[8..16], &unchecked[8..16]);
            assert_eq!(&checked[16..], &timestamp[16..]);
            assert_eq!(&unchecked[16..], &timestamp[16..]);
            for (name, actual) in [("checked", checked), ("unchecked", unchecked)] {
                let expected =
                    fs_err::read(directory.path().join(format!("{index}.py.{name}"))).unwrap();
                assert_eq!(
                    actual,
                    expected,
                    "{python_version} {name} source case {index} ({} bytes)",
                    source.len()
                );
            }
        }
    }
}
