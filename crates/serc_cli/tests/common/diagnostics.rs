use serc::{
    CompileError, CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options,
};

const COMPILE_DIAGNOSTIC: &str = r"
import marshal, sys
try:
    code = compile(sys.argv[1], sys.argv[2], 'exec', dont_inherit=True, optimize=0)
except SyntaxError as error:
    print(error.msg)
    raise SystemExit(1)
sys.stdout.buffer.write(marshal.dumps(code))
";

const SYNTAX_DIAGNOSTIC: &str = r"
import sys
diagnostic = None
sources = (sys.argv[1], sys.argv[1].encode()) if sys.argv[2] == 'bytes' else (sys.argv[1],)
for source in sources:
    try:
        compile(source, 'case.py', 'exec', dont_inherit=True)
    except SyntaxError as error:
        actual = (error.msg, error.lineno, error.offset, error.end_lineno, error.end_offset)
        assert diagnostic is None or diagnostic == actual
        diagnostic = actual
    else:
        raise AssertionError('expected a syntax error')
print(diagnostic[0])
print(*diagnostic[1:])
";

/// Compare unoptimized text, byte, and CLI compilation with CPython.
/// Accepted sources must also produce byte-identical marshal output.
pub(crate) fn assert_compilation(
    source: &str,
    path: &std::path::Path,
    version: PythonVersion,
    expected: Option<&str>,
) {
    let options = CompileOptions {
        python_version: version,
        ..CompileOptions::default()
    };
    let mut python = super::python_command(version);
    let native = python
        .args(["-c", COMPILE_DIAGNOSTIC, source])
        .arg(path)
        .output()
        .unwrap();
    assert_eq!(
        native.status.success(),
        expected.is_none(),
        "{version}: {source}: {native:?}"
    );
    for compiled in [
        compile_with_options(source, path.to_str().unwrap(), &options),
        compile_bytes_with_options(source.as_bytes(), path.to_str().unwrap(), &options),
    ] {
        if let Some(expected) = expected {
            assert_eq!(String::from_utf8_lossy(&native.stdout).trim(), expected);
            let error = compiled.unwrap_err();
            assert!(
                error.to_string().ends_with(expected),
                "{version}: {source}: {error}"
            );
        } else {
            assert_eq!(
                compiled.unwrap().marshal(),
                native.stdout,
                "{version}: {source}"
            );
        }
    }
    fs_err::write(path, source).unwrap();
    let output = super::check(path, version);
    assert_eq!(
        output.status.success(),
        expected.is_none(),
        "{version}: {source}: {output:?}"
    );
    if let Some(expected) = expected {
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{output:?}"
        );
    }
}

pub(crate) struct Diagnostic {
    pub(crate) message: String,
    positions: [usize; 4],
}

/// Parse the message and start/end coordinates emitted by a diagnostic oracle.
pub(crate) fn parse_diagnostic(stdout: Vec<u8>) -> Diagnostic {
    let stdout = String::from_utf8(stdout).unwrap();
    let mut lines = stdout.lines();
    let message = lines.next().unwrap().to_owned();
    let positions: Vec<usize> = lines
        .next()
        .unwrap()
        .split_whitespace()
        .map(|value| value.parse().unwrap())
        .collect();
    Diagnostic {
        message,
        positions: positions.try_into().unwrap(),
    }
}

/// Obtain CPython's diagnostic for a string source compiled as `case.py`.
pub(crate) fn syntax_diagnostic(version: PythonVersion, source: &str) -> Diagnostic {
    run_oracle(version, source, "str")
}

/// Also require the byte-source diagnostic to agree with the string-source diagnostic.
pub(crate) fn syntax_diagnostic_with_bytes(version: PythonVersion, source: &str) -> Diagnostic {
    run_oracle(version, source, "bytes")
}

fn run_oracle(version: PythonVersion, source: &str, mode: &str) -> Diagnostic {
    let output = super::python_command(version)
        .args(["-c", SYNTAX_DIAGNOSTIC, source, mode])
        .output()
        .unwrap();
    assert!(output.status.success(), "{version}: {source:?}: {output:?}");
    parse_diagnostic(output.stdout)
}

impl Diagnostic {
    /// Convert symbol-table columns, which count UTF-8 bytes, to source offsets.
    pub(crate) fn byte_range(&self, source: &str) -> (usize, usize) {
        self.range(source, |_, column| column - 1)
    }

    /// Convert parser columns, which count Unicode characters, to source byte offsets.
    pub(crate) fn character_range(&self, source: &str) -> (usize, usize) {
        self.range(source, |line, column| {
            line.char_indices()
                .nth(column - 1)
                .map_or(line.len(), |(offset, _)| offset)
        })
    }

    fn range(&self, source: &str, column_offset: impl Fn(&str, usize) -> usize) -> (usize, usize) {
        let offset = |line: usize, column: usize| {
            let prefix = source
                .split_inclusive('\n')
                .take(line - 1)
                .map(str::len)
                .sum::<usize>();
            prefix + column_offset(source[prefix..].split('\n').next().unwrap(), column)
        };
        let [start_line, start_column, end_line, end_column] = self.positions;
        (
            offset(start_line, start_column),
            offset(end_line, end_column),
        )
    }

    /// Check the diagnostic display message and range from both compiler entry points.
    pub(crate) fn assert_error(
        &self,
        source: &str,
        options: &CompileOptions,
        range: (usize, usize),
    ) {
        Self::assert_matches(source, options, range, |error| {
            assert!(
                error.to_string().ends_with(&self.message),
                "{}: {source:?}: {:?} != {error}",
                options.python_version,
                self.message
            );
        });
    }

    /// Additionally require the compiler's error to be the exact `InvalidSyntax` variant.
    pub(crate) fn assert_syntax_error(
        &self,
        source: &str,
        options: &CompileOptions,
        range: (usize, usize),
    ) {
        Self::assert_matches(source, options, range, |error| {
            let CompileError::InvalidSyntax { message, .. } = error else {
                panic!(
                    "{}: {source:?}: expected syntax error, got {error}",
                    options.python_version
                );
            };
            assert_eq!(
                message, &self.message,
                "{}: {source:?}",
                options.python_version
            );
        });
    }

    fn assert_matches(
        source: &str,
        options: &CompileOptions,
        expected: (usize, usize),
        assert_message: impl Fn(&CompileError),
    ) {
        for result in [
            compile_with_options(source, "case.py", options),
            compile_bytes_with_options(source.as_bytes(), "case.py", options),
        ] {
            let error = result.unwrap_err();
            assert_message(&error);
            let range = error.range().unwrap();
            assert_eq!(
                (range.start().to_usize(), range.end().to_usize()),
                expected,
                "{}: {source:?}: {error}",
                options.python_version
            );
        }
    }
}
