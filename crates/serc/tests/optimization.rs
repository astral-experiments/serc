use std::ffi::OsString;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

use serc::{
    CompileOptions, OptimizationLevel, PythonVersion, compile, compile_bytes_with_options,
    compile_bytes_with_path_and_options, compile_with_options,
};

const VERSIONS: [(PythonVersion, &str, &str); 4] = [
    (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
    (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
    (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
    (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
];
const LEVELS: [OptimizationLevel; 3] = [
    OptimizationLevel::Zero,
    OptimizationLevel::One,
    OptimizationLevel::Two,
];

static ORACLES: [OnceLock<OsString>; 4] = [const { OnceLock::new() }; 4];

const ORACLE_IDENTITY: &str = r"
import importlib.util, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
";
const MARSHAL_ORACLE: &str = r"
import marshal, sys
code = compile(sys.stdin.buffer.read(), sys.argv[2], 'exec', dont_inherit=True, optimize=int(sys.argv[1]))
sys.stdout.buffer.write(marshal.dumps(code))
";

fn oracle(
    source: &[u8],
    filename: &str,
    version: PythonVersion,
    level: OptimizationLevel,
) -> Output {
    let index = VERSIONS
        .iter()
        .position(|(candidate, _, _)| *candidate == version)
        .unwrap();
    let executable = ORACLES[index].get_or_init(|| {
        let (_, variable, fallback) = VERSIONS[index];
        let executable = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let target = version.target();
        let identity = Command::new(&executable)
            .args(["-I", "-S", "-c", ORACLE_IDENTITY])
            .args([target.tag.trim_start_matches('v'), &target.magic_hex()])
            .output()
            .expect("check the pinned CPython oracle");
        assert!(
            identity.status.success(),
            "{variable} must name {} with magic {}: {}",
            target.tag,
            target.magic_hex(),
            String::from_utf8_lossy(&identity.stderr)
        );
        executable
    });
    let level = match level {
        OptimizationLevel::Zero => "0",
        OptimizationLevel::One => "1",
        OptimizationLevel::Two => "2",
    };
    let mut child = Command::new(executable)
        .args(["-I", "-S", "-X", "utf8", "-c", MARSHAL_ORACLE])
        .args([level, filename])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the pinned CPython oracle");
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}

fn assert_matches_cpython(source: &str, filename: &str, levels: &[OptimizationLevel]) {
    for (python_version, _, _) in VERSIONS {
        for &optimization_level in levels {
            let options = CompileOptions {
                python_version,
                optimization_level,
                ..CompileOptions::default()
            };
            let expected = oracle(
                source.as_bytes(),
                filename,
                python_version,
                optimization_level,
            );
            assert!(
                expected.status.success(),
                "{python_version}, {optimization_level:?}, {filename}: {}",
                String::from_utf8_lossy(&expected.stderr)
            );
            let actual = compile_with_options(source, filename, &options)
                .unwrap_or_else(|error| {
                    panic!("{python_version}, {optimization_level:?}, {filename}: {error}")
                })
                .marshal();
            let first_difference = actual
                .iter()
                .zip(&expected.stdout)
                .position(|(actual, expected)| actual != expected);
            assert!(
                actual == expected.stdout,
                "{python_version}, {optimization_level:?}, {filename}: first difference \
                 {first_difference:?}, lengths {} and {}",
                actual.len(),
                expected.stdout.len()
            );
        }
    }
}

#[test]
fn assertions_match_cpython_at_every_optimization_level() {
    assert_matches_cpython(
        r"
assert ready, describe()
def check(value):
    assert value
    assert value > 0, f'bad value: {value}'
    return value
def only_assert():
    assert side_effect(), message()
class C:
    assert ready
    def method(self):
        assert self.ready
try:
    effect()
finally:
    assert ready
try:
    assert ready
except ValueError:
    assert recovery
try:
    assert ready
except:
    assert recovery
after = 1
try:
    assert ready
except:
    assert recovery
finally:
    after = 2
",
        "optimized_assertions.py",
        &LEVELS,
    );
}

#[test]
fn removed_assertions_preserve_scope_and_generator_metadata() {
    assert_matches_cpython(
        r"
def assigned():
    assert (local := value)
    return local
def outer(captured):
    def inner():
        assert captured
    return inner
def captured_by_assert(local):
    assert lambda: local
def generator():
    assert (yield value)
def delegated():
    assert (yield from values)
async def async_generator():
    assert (yield value)
async def coroutine():
    assert await value
",
        "optimized_assertion_scopes.py",
        &LEVELS,
    );
}

#[test]
fn debug_constants_match_cpython_through_nested_expressions_and_scopes() {
    assert_matches_cpython(
        r"
debug = __debug__
constants = (__debug__, not __debug__, (__debug__,))
choice = 'debug' if __debug__ else 'optimized'
if __debug__:
    branch = 1
else:
    branch = 2
def function(first):
    while __debug__:
        break
    if not __debug__:
        first += 1
    return __debug__
def conjunction(value):
    return __debug__ and value
def disjunction(value):
    return __debug__ or value
class C:
    debug = __debug__
    def method(self, default=__debug__):
        return lambda: (__debug__, default)
values = [__debug__ for item in items if __debug__ or item]
generator = (__debug__ for item in items if not __debug__)
",
        "optimized_debug.py",
        &LEVELS,
    );
}

#[test]
fn debug_annotations_preserve_postponed_source_spelling() {
    let source =
        "value: __debug__ = 1\ndef function(arg: __debug__) -> tuple[__debug__]:\n    return arg\n";
    assert_matches_cpython(source, "optimized_annotations.py", &LEVELS);
    assert_matches_cpython(
        "assert condition\nx: int = 1\n",
        "optimized_assertion_annotation.py",
        &LEVELS,
    );
    assert_matches_cpython(
        &format!("from __future__ import annotations\n{source}"),
        "optimized_future_annotations.py",
        &LEVELS,
    );
}

#[test]
fn removed_assertion_comprehensions_do_not_allocate_temporary_locals() {
    assert_matches_cpython(
        r"
assert [item for item in values]
class C:
    assert [item for item in values]
def check(values):
    assert [item for item in values]
    after = 1
    return after
async def async_check(values):
    assert [item async for item in values]
def lexical_before(values):
    item = 1
    assert [item for item in values]
    return item
def lexical_after(values):
    assert [item for item in values]
    after = 1
    item = 2
    return (after, item)
def captured_comprehension(values):
    assert [lambda: item for item in values]
def asserted_local_reference():
    assert later
    first = 1
    later = 2
    return (first, later)
def unused_assertion_binding():
    assert (unused := 1)
    used = 2
    return used
",
        "optimized_comprehension_locals.py",
        &LEVELS,
    );
}

#[test]
fn removed_assertion_comprehensions_preserve_module_and_class_cells() {
    assert_matches_cpython(
        r"
assert [lambda: item for item in values]
class C:
    assert [lambda: item for item in values]
",
        "optimized_comprehension_cells.py",
        &LEVELS,
    );
    assert_matches_cpython(
        r"
assert [lambda: _ for _ in values]
x: int = 1
class C:
    assert [lambda: _ for _ in values]
    def method(self):
        return __class__
",
        "optimized_comprehension_cell_order.py",
        &LEVELS,
    );
}

#[test]
fn comprehension_temporaries_follow_surviving_execution_order() {
    assert_matches_cpython(
        r"
assert [x for x in a]
[y for y in b]
[x for x in c]
class C:
    assert [x for x in a]
    [y for y in b]
    [x for x in c]
",
        "optimized_comprehension_order.py",
        &LEVELS,
    );
}

#[test]
fn stripped_docstrings_match_cpython_flags_constants_and_locations() {
    for (source, filename) in [
        ("\"module docstring\"\n", "optimized_module_docstring.py"),
        (
            "\"doc\"\n\"\\ud800\"\ndef f():\n    \"doc\"\n    \"\"\n    x=1\n",
            "optimized_surrogate_after_docstring.py",
        ),
        ("\"doc\"\nx: int = 1\n", "optimized_docstring_annotation.py"),
        (
            "\"doc\"\nfrom __future__ import annotations\nx: int = 1\n",
            "optimized_docstring_future_annotation.py",
        ),
        (
            r#""module docstring"
"a later string expression"
def function():
    "function docstring"
    "another string expression"
    return "value"
def empty():
    "only a docstring"
async def coroutine():
    "coroutine docstring"
    return 1
def generator():
    "generator docstring"
    yield 1
class C:
    "class docstring"
    def method(self):
        "method docstring"
        return __class__
    class Empty:
        "only a nested class docstring"
"#,
            "optimized_docstrings.py",
        ),
        (
            r#"def outer():
    "retained value"
    def inner():
        "retained value"
        return "retained value"
    return inner
"#,
            "optimized_docstring_identity.py",
        ),
    ] {
        assert_matches_cpython(source, filename, &LEVELS);
    }
}

#[test]
fn removed_assertions_do_not_create_docstrings() {
    assert_matches_cpython(
        r#"assert True
"not a module docstring"
def function():
    assert True
    "not a function docstring"
class C:
    assert True
    "not a class docstring"
"#,
        "optimized_non_docstrings.py",
        &LEVELS,
    );
}

#[test]
fn optimized_assertion_syntax_matches_the_target_version() {
    for source in [
        "assert (await value)\n",
        "assert (__debug__ := False)\n",
        "def f():\n    assert (value := 1)\n    global value\n",
    ] {
        for (python_version, _, _) in VERSIONS {
            for optimization_level in LEVELS {
                let expected = oracle(
                    source.as_bytes(),
                    "invalid_optimized_assert.py",
                    python_version,
                    optimization_level,
                );
                let options = CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                };
                let actual = compile_with_options(source, "invalid_optimized_assert.py", &options);
                if expected.status.success() {
                    assert_eq!(
                        actual.unwrap().marshal(),
                        expected.stdout,
                        "{python_version}, {optimization_level:?}, {source:?}"
                    );
                } else {
                    assert!(
                        String::from_utf8_lossy(&expected.stderr).contains("SyntaxError"),
                        "the pinned oracle failed unexpectedly: {}",
                        String::from_utf8_lossy(&expected.stderr)
                    );
                    assert!(
                        actual.is_err(),
                        "{python_version}, {optimization_level:?} accepted {source:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn optimized_assertions_skip_codegen_only_syntax_errors() {
    assert_matches_cpython(
        "assert (yield 1)\n",
        "optimized_module_yield.py",
        &[OptimizationLevel::One, OptimizationLevel::Two],
    );
}

#[test]
fn source_decoding_preserves_optimization_options() {
    let source = b"# coding: latin-1\r\n\"caf\xe9\"\r\nassert __debug__\r\nvalue = __debug__\r\n";
    let filename = "optimized_encoding.py";
    for (python_version, _, _) in VERSIONS {
        for optimization_level in LEVELS {
            let options = CompileOptions {
                python_version,
                optimization_level,
                ..CompileOptions::default()
            };
            let expected = oracle(source, filename, python_version, optimization_level);
            assert!(
                expected.status.success(),
                "{}",
                String::from_utf8_lossy(&expected.stderr)
            );
            for actual in [
                compile_bytes_with_options(source, filename, &options).unwrap(),
                compile_bytes_with_path_and_options(source, Path::new(filename), &options).unwrap(),
            ] {
                assert_eq!(
                    actual.marshal(),
                    expected.stdout,
                    "{python_version}, {optimization_level:?}"
                );
            }
        }
    }
}

#[test]
fn default_options_preserve_unoptimized_behavior() {
    let source = "\"module docstring\"\nassert condition\nvalue = __debug__\n";
    let default = CompileOptions::default();
    assert_eq!(default.optimization_level, OptimizationLevel::Zero);
    assert_eq!(
        compile(source, "default_optimization.py")
            .unwrap()
            .marshal(),
        compile_with_options(source, "default_optimization.py", &default)
            .unwrap()
            .marshal()
    );
}

#[test]
fn optimized_deep_expression_traversal_and_cleanup() {
    const CHILD_ENV: &str = "SERC_TEST_OPTIMIZED_DEEP_EXPRESSIONS";
    if std::env::var_os(CHILD_ENV).is_some() {
        let expression = std::iter::repeat_n("value", 2_048)
            .collect::<Vec<_>>()
            .join(" + ");
        let source = format!(
            "\"module docstring\"\nvalue = 1\nassert {expression}\nresult = __debug__ + {expression}\n"
        );
        for optimization_level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let options = CompileOptions {
                optimization_level,
                ..CompileOptions::default()
            };
            let module = compile_with_options(&source, "deep_optimized.py", &options).unwrap();
            assert!(!module.marshal().is_empty());
        }
        return;
    }

    // A stack-overflow regression must fail this test without aborting the entire test binary.
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "optimized_deep_expression_traversal_and_cleanup",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "optimized expression subprocess failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
