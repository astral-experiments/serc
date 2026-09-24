use serc::{
    CompileError, CompileOptions, PythonVersion, compile, compile_bytes,
    compile_bytes_with_options, compile_with_options,
};

use crate::common::{assert_matches_cpython_marshal, python_3145_command};

#[test]
fn rejects_semantic_errors_before_discarding_code() {
    for source in [
        "if False:\n    break\n",
        "type X = (y := x for x in xs)\n",
        "type X = (x for x in await xs)\n",
        "type X = lambda: await x\n",
        "if False:\n    return 1\n",
        "def f():\n    if False:\n        await x\n",
        "while False:\n    def f():\n        break\n",
        "def f():\n    class C:\n        return 1\n",
        "if False:\n    from __future__ import annotations\n",
        "if False:\n    def f(x, x): pass\n",
        "if False:\n    match x:\n        case [a] | [b]: pass\n",
        "def f():\n    return\n    async for x in xs: pass\n",
        "def f():\n    return\n    values = [x for x in xs if await x]\n",
        "from __future__ import annotations\ndef f(x: (lambda y=(yield 1): y)): pass\n",
    ] {
        let expected = python_3145_command()
            .args([
                "-c",
                "import sys; compile(sys.argv[1], 'semantic.py', 'exec')",
                source,
            ])
            .output()
            .unwrap();
        assert!(!expected.status.success(), "CPython accepted {source}");
        assert!(String::from_utf8_lossy(&expected.stderr).contains("SyntaxError:"));
        let error = compile(source, "semantic.py").unwrap_err();
        assert!(
            matches!(
                error,
                CompileError::InvalidSyntax { .. } | CompileError::InvalidPattern { .. }
            ),
            "expected a source diagnostic for {source}, got {error:?}"
        );
        assert!(error.range().is_some_and(|range| !source[range].is_empty()));
    }
}

#[test]
fn semantic_validation_respects_scope_boundaries() {
    for source in [
        "from .__future__ import annotations\nx = 1\n",
        "def f():\n    yield 1\n    def g(x=(yield 2)): pass\n",
        "async def f():\n    def g(x=await value): pass\n",
        "def f():\n    return (await x for x in xs)\n",
        "def f():\n    return (x async for x in xs)\n",
        "async def f():\n    return [[x async for x in xs] for xs in groups]\n",
        "from __future__ import annotations\ndef outer():\n    def inner(x: (lambda: (yield 1))): pass\n",
        "type X = lambda: (yield 1)\n",
        "type X = lambda: (x := 1)\n",
        "type X = lambda: (await x for x in xs)\n",
        "type X = (await x for x in xs)\n",
        "type X = (x async for x in xs)\n",
        "from __future__ import annotations\ndef f(x: (await y for y in ys)): pass\n",
        "def f[T: (await x for x in xs)](): pass\n",
        "def f[T: (lambda: (yield 1))](): pass\n",
        "while outer:\n    while inner: pass\n    else: break\n",
    ] {
        assert_matches_cpython_marshal(source, "semantic.py");
    }
}

#[test]
fn unreachable_or_pattern_arms_match_cpython_diagnostics() {
    let sources = [
        "match subject:\n    case [value | value]: pass\n",
        "match subject:\n    case [value | 1]: pass\n",
        "match subject:\n    case [_ | 1]: pass\n",
        "match subject:\n    case {'field': (value | value)}: pass\n",
        "match subject:\n    case Point(value | value): pass\n",
        "match subject:\n    case Point(field=(_ | 1)): pass\n",
        "match subject:\n    case [((value | value) as whole)]: pass\n",
        "match subject:\n    case [(_ as value) | (_ as value)]: pass\n",
        "match subject:\n    case [(1 | _) | 2]: pass\n",
        "match subject:\n    case ((1 | _) as value) | (2 as value): pass\n",
        "match subject:\n    case [value | value] if condition: pass\n    case _: pass\n",
        "match subject:\n    case first: pass\n    case [second | second]: pass\n",
        "match subject:\n    case [first | first]: pass\n    case second: pass\n    case 3: pass\n",
        "match subject:\n    case [\n        value\n        | value\n    ]: pass\n",
        "match subject:\n    case [\u{212a} | K]: pass\n",
        "if False:\n    match subject:\n        case [value | value]: pass\n",
        "def f():\n    return\n    match subject:\n        case [value | value]: pass\n",
    ];
    let oracle = r"
import json, sys
try:
    compile(sys.argv[1], 'semantic.py', 'exec')
except SyntaxError as error:
    print(json.dumps([error.msg, error.lineno, error.offset, error.end_lineno, error.end_offset]))
else:
    raise SystemExit('CPython unexpectedly accepted the source')
";

    for source in sources {
        let output = python_3145_command()
            .args(["-c", oracle, source])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let configurations = [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ];
        let results = configurations
            .into_iter()
            .map(|python_version| {
                compile_with_options(
                    source,
                    "semantic.py",
                    &CompileOptions {
                        python_version,
                        ..CompileOptions::default()
                    },
                )
            })
            .chain([compile_bytes(source.as_bytes(), "semantic.py")]);
        for result in results {
            let error = result.unwrap_err();
            let CompileError::InvalidPattern { message, range } = &error else {
                panic!("expected InvalidPattern for {source:?}, got {error:?}");
            };
            assert_eq!(error.range(), Some(*range));
            let (line, column) = source_position(source, usize::from(range.start()));
            let (end_line, end_column) = source_position(source, usize::from(range.end()));
            assert_eq!(
                serde_json::json!([message, line, column, end_line, end_column]),
                expected,
                "{source}"
            );
        }
    }
}

fn source_position(source: &str, offset: usize) -> (usize, usize) {
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|&byte| byte == b'\n').count() + 1;
    let column = prefix.rsplit('\n').next().unwrap().len() + 1;
    (line, column)
}

#[test]
fn final_irrefutable_or_arms_keep_cpython_bytecode() {
    for source in [
        "match subject:\n    case [1 | _]: pass\n",
        "match subject:\n    case [(1 | _)] | [2]: pass\n",
        "match subject:\n    case {'field': (1 | _)}: pass\n",
        "match subject:\n    case {'field': (1 | _)} | [2]: pass\n",
        "match subject:\n    case Point(field=(1 | _)): pass\n",
        "match subject:\n    case Point(1 | _) | Point(2): pass\n",
        "match subject:\n    case [([value] | value)]: pass\n",
        "match subject:\n    case ((1 as value) | value): pass\n",
        "match subject:\n    case (1 as value) | (_ as value): pass\n",
        "match subject:\n    case ([value] | value) if condition: pass\n    case _: pass\n",
        "match subject:\n    case [([value] | value)] | [([value] | value)]: pass\n",
    ] {
        assert_matches_cpython_marshal(source, "semantic.py");
    }
}

#[test]
fn unreachable_or_arms_preserve_cpython_pattern_error_precedence() {
    for (pattern, diagnostic) in [
        ("[value | 1, __debug__]", "cannot assign to"),
        ("[value | 1, *left, *right]", "multiple starred names"),
        ("{'field': value | 1, 'field': 2}", "duplicate key"),
        (
            "Point(value | 1, field=1, field=2)",
            "repeated in class pattern",
        ),
        ("[first, first, value | 1]", "multiple assignments to name"),
        (
            "[value | 1, second, second]",
            "makes remaining patterns unreachable",
        ),
        (
            "(1 as first) | (2 as second) | (third | 3)",
            "alternative patterns bind different names",
        ),
        (
            "(1 as first) | (second | 2)",
            "makes remaining patterns unreachable",
        ),
    ] {
        let source = format!("match subject:\n    case {pattern}: pass\n");
        let expected = python_3145_command()
            .args([
                "-c",
                "import sys; compile(sys.argv[1], 'semantic.py', 'exec')",
                &source,
            ])
            .output()
            .unwrap();
        assert!(!expected.status.success(), "CPython accepted {source}");
        assert!(
            String::from_utf8_lossy(&expected.stderr).contains(diagnostic),
            "CPython {source}: {}",
            String::from_utf8_lossy(&expected.stderr)
        );
        let error = compile(&source, "semantic.py").unwrap_err();
        assert!(
            error.to_string().contains(diagnostic),
            "Serc {source}: {error}"
        );
    }
}

#[test]
fn declarations_precede_unreachable_pattern_lowering_but_not_existing_symbol_errors() {
    for (source, diagnostic) in [
        (
            "def f():\n    match subject:\n        case [x | x]: pass\n    value = 1\n    global value\n",
            "name 'value' is assigned to before global declaration",
        ),
        (
            "def f():\n    nonlocal value\n    match subject:\n        case [x | x]: pass\n",
            "no binding for nonlocal 'value' found",
        ),
        (
            "def f():\n    match subject:\n        case [x | x]: pass\n    nonlocal value\n",
            "no binding for nonlocal 'value' found",
        ),
        (
            "match subject:\n    case [x | x]: pass\nvalue = 1\nglobal value\n",
            "name 'value' is assigned to before global declaration",
        ),
        (
            "def f():\n    match subject:\n        case [x | x, __debug__]: pass\n    value = 1\n    global value\n",
            "cannot assign to",
        ),
    ] {
        let expected = python_3145_command()
            .args([
                "-c",
                "import sys; compile(sys.argv[1], 'semantic.py', 'exec')",
                source,
            ])
            .output()
            .unwrap();
        assert!(!expected.status.success(), "CPython accepted {source}");
        assert!(String::from_utf8_lossy(&expected.stderr).contains(diagnostic));
        for python_version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let options = CompileOptions {
                python_version,
                ..CompileOptions::default()
            };
            let error = compile_with_options(source, "semantic.py", &options).unwrap_err();
            let CompileError::InvalidSyntax { message, .. } = &error else {
                panic!("expected a symbol diagnostic for {source:?}, got {error:?}");
            };
            assert!(
                message.contains(diagnostic),
                "{python_version:?}: {source}: {message}"
            );
        }
    }
}

const STARRED_TARGETS: [PythonVersion; 4] = [
    PythonVersion::Py312,
    PythonVersion::Py313,
    PythonVersion::Py314,
    PythonVersion::Py315,
];

fn assert_rejects_raw_starred_expression(
    source: &str,
    highlighted: &str,
    expected_message: &str,
    versions: &[PythonVersion],
) {
    let start = source.find(highlighted).unwrap();
    assert_selected_semantic_diagnostic(
        source,
        start..start + highlighted.len(),
        expected_message,
        versions,
        false,
    );
}

fn assert_selected_semantic_diagnostic(
    source: &str,
    expected_range: std::ops::Range<usize>,
    expected_message: &str,
    versions: &[PythonVersion],
    pattern_error: bool,
) {
    let expected = python_3145_command()
        .args([
            "-c",
            r"
import json, sys

source = sys.argv[1]
lines = source.splitlines(keepends=True)

def absolute(lineno, offset):
    return sum(len(line.encode()) for line in lines[:lineno - 1]) + offset - 1

def diagnostic(value):
    try:
        compile(value, 'starred.py', 'exec', dont_inherit=True)
    except SyntaxError as error:
        return [error.msg, absolute(error.lineno, error.offset),
                absolute(error.end_lineno, error.end_offset)]
    raise AssertionError('CPython accepted the source')

print(json.dumps([diagnostic(source), diagnostic(source.encode())]))
",
            source,
        ])
        .output()
        .unwrap();
    assert!(
        expected.status.success(),
        "CPython failed for {source:?}: {}",
        String::from_utf8_lossy(&expected.stderr)
    );
    let oracle: [(String, usize, usize); 2] = serde_json::from_slice(&expected.stdout).unwrap();
    for (message, start, end) in oracle {
        assert_eq!(message, expected_message, "CPython: {source:?}");
        assert_eq!(start..end, expected_range, "CPython: {source:?}");
    }

    for &python_version in versions {
        let options = CompileOptions {
            python_version,
            ..CompileOptions::default()
        };
        for result in [
            compile_with_options(source, "starred.py", &options),
            compile_bytes_with_options(source.as_bytes(), "starred.py", &options),
        ] {
            let error = result.expect_err(source);
            let (message, range) = match (&error, pattern_error) {
                (CompileError::InvalidSyntax { message, range }, false)
                | (CompileError::InvalidPattern { message, range }, true) => (message, range),
                _ => panic!("unexpected semantic error for {python_version} {source:?}: {error:?}"),
            };
            assert_eq!(message, expected_message, "{python_version}: {source:?}");
            assert_eq!(
                usize::from(range.start())..usize::from(range.end()),
                expected_range,
                "{python_version}: {source:?}"
            );
        }
    }
}

#[test]
fn declarations_precede_raw_stars_but_pattern_errors_compete_in_source_order() {
    const STAR: &str = "can't use starred expression here";
    const WILDCARD: &str = "wildcard makes remaining patterns unreachable";
    const CAPTURE: &str = "name capture 'capture' makes remaining patterns unreachable";
    const DUPLICATE: &str = "multiple assignments to name 'capture' in pattern";

    for (source, highlighted, message, pattern_error) in [
        (
            "def f():\n    *values\n    value = 1\n    global value\n",
            "global value",
            "name 'value' is assigned to before global declaration",
            false,
        ),
        (
            "def f():\n    value = 1\n    global value\n    *values\n",
            "global value",
            "name 'value' is assigned to before global declaration",
            false,
        ),
        (
            "def f(value):\n    *values\n    global value\n",
            "global value",
            "name 'value' is parameter and global",
            false,
        ),
        (
            "def f(value):\n    global value\n    *values\n",
            "global value",
            "name 'value' is parameter and global",
            false,
        ),
        (
            "def f():\n    *values\n    nonlocal value\n",
            "nonlocal value",
            "no binding for nonlocal 'value' found",
            false,
        ),
        (
            "def f():\n    nonlocal value\n    *values\n",
            "nonlocal value",
            "no binding for nonlocal 'value' found",
            false,
        ),
        (
            "*values\nmatch subject:\n    case _ | 1: pass\n",
            "*values",
            STAR,
            false,
        ),
        (
            "match subject:\n    case _ | 1: pass\n*values\n",
            "_",
            WILDCARD,
            true,
        ),
        (
            "*values\nmatch subject:\n    case capture | 1: pass\n",
            "*values",
            STAR,
            false,
        ),
        (
            "match subject:\n    case capture | 1: pass\n*values\n",
            "capture",
            CAPTURE,
            true,
        ),
        (
            "*values\nmatch subject:\n    case [capture, capture]: pass\n",
            "*values",
            STAR,
            false,
        ),
        (
            "match subject:\n    case [capture, capture]: pass\n*values\n",
            "capture",
            DUPLICATE,
            true,
        ),
        (
            "match f\"{*values}\":\n    case _ | 1: pass\n",
            "*values",
            STAR,
            false,
        ),
        (
            "match subject:\n    case 1 if f\"{*values}\": pass\n    case _ | 2: pass\n",
            "*values",
            STAR,
            false,
        ),
        (
            "match subject:\n    case 1:\n        *values\n    case _ | 2: pass\n",
            "*values",
            STAR,
            false,
        ),
        (
            "match subject:\n    case _ | 1 if f\"{*values}\": pass\n",
            "_",
            WILDCARD,
            true,
        ),
    ] {
        let start = source.rfind(highlighted).unwrap();
        assert_selected_semantic_diagnostic(
            source,
            start..start + highlighted.len(),
            message,
            &STARRED_TARGETS,
            pattern_error,
        );
    }
}

#[test]
fn rejects_raw_starred_values_before_discarding_code() {
    const MESSAGE: &str = "can't use starred expression here";
    for (source, highlighted) in [
        ("*values\n", "*values"),
        ("pass\n*values\n", "*values"),
        ("é = 1; *values\n", "*values"),
        ("def f():\n    *values\n", "*values"),
        ("async def f():\n    *values\n", "*values"),
        ("class C:\n    *values\n", "*values"),
        ("if False:\n    *values\n", "*values"),
        ("while False:\n    *values\n", "*values"),
        ("def f():\n    return\n    *values\n", "*values"),
        ("* (\n    values\n)\n", "* (\n    values\n)"),
        ("*(first + second)\n", "*(first + second)"),
        ("value: object = *values\n", "*values"),
        ("obj.value: object = *values\n", "*values"),
        ("def f():\n    value: object = *values\n", "*values"),
        (
            "from __future__ import annotations\nvalue: object = *values\n",
            "*values",
        ),
        ("result = f'{*values}'\n", "*values"),
        ("result = f'{*values!r}'\n", "*values"),
        ("result = f'{value:{*values}}'\n", "*values"),
        ("if False:\n    result = f'{*values}'\n", "*values"),
        ("result = False and f'{*values}'\n", "*values"),
        ("for item in *values: pass\n", "*values"),
        ("for *item in *values: pass\n", "*values"),
        ("for *item in f'{*values}': pass\n", "*values"),
        ("first = *second = f'{*values}'\n", "*values"),
    ] {
        assert_rejects_raw_starred_expression(source, highlighted, MESSAGE, &STARRED_TARGETS);
    }

    for source in [
        "result = t'{*values}'\n",
        "result = t'{value:{*values}}'\n",
        "if False:\n    result = t'{*values}'\n",
    ] {
        assert_rejects_raw_starred_expression(
            source,
            "*values",
            MESSAGE,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn rejects_raw_starred_targets_in_chained_assignments_and_with() {
    const MESSAGE: &str = "starred assignment target must be in a list or tuple";
    for (source, highlighted) in [
        ("*first = second = values\n", "*first"),
        ("first = *second = values\n", "*second"),
        ("*first = *second = values\n", "*first"),
        ("*obj.first = second = values\n", "*obj.first"),
        ("if False:\n    first = *second = values\n", "*second"),
        ("with manager as *target:\n    pass\n", "*target"),
        (
            "with first as valid, second as *target:\n    pass\n",
            "*target",
        ),
        ("with (manager as *target):\n    pass\n", "*target"),
        (
            "async def f():\n    async with manager as *target:\n        pass\n",
            "*target",
        ),
        ("if False:\n    with manager as *target: pass\n", "*target"),
        ("for *target in values: pass\n", "*target"),
        ("if False:\n    for *target in values: pass\n", "*target"),
        (
            "async def f():\n    async for *target in values: pass\n",
            "*target",
        ),
    ] {
        assert_rejects_raw_starred_expression(source, highlighted, MESSAGE, &STARRED_TARGETS);
    }
}

fn assert_accepts_valid_starred_expression(source: &str, versions: &[PythonVersion]) {
    assert_matches_cpython_marshal(source, "starred.py");
    for &python_version in versions {
        let options = CompileOptions {
            python_version,
            ..CompileOptions::default()
        };
        let text = compile_with_options(source, "starred.py", &options)
            .unwrap_or_else(|error| panic!("{python_version}: {source:?}: {error:?}"));
        let bytes = compile_bytes_with_options(source.as_bytes(), "starred.py", &options)
            .unwrap_or_else(|error| panic!("{python_version}: {source:?}: {error:?}"));
        assert_eq!(
            text.marshal(),
            bytes.marshal(),
            "{python_version}: {source:?}"
        );
    }
}

#[test]
fn preserves_valid_starred_unpacking_and_unevaluated_annotations() {
    for source in [
        "*values,\n",
        "(*values,)\n",
        "[*values]\n",
        "{*values}\n",
        "call(*values)\n",
        "obj[*values]\n",
        "class C(*values): pass\n",
        "first = (*target,) = values\n",
        "(*target,) = second = values\n",
        "first = [*target] = values\n",
        "with manager as (*target,): pass\n",
        "with manager as [*target]: pass\n",
        "for (*target,) in values: pass\n",
        "for [*target] in values: pass\n",
        "for item in (*values,): pass\n",
        "async def f():\n    async for (*target,) in values: pass\n",
        "value: object = (*values,)\n",
        "result = f'{*values,}'\n",
        "result = f'{value:{(*values,)}}'\n",
        "def f(*args: *values): pass\n",
        "from __future__ import annotations\nvalue: f'{*values}' = 1\n",
        "def f():\n    value: f'{*values}' = 1\n",
    ] {
        assert_accepts_valid_starred_expression(source, &STARRED_TARGETS);
    }

    for source in [
        "result = t'{*values,}'\n",
        "result = t'{value:{(*values,)}}'\n",
        "obj.attr: f'{*values}' = 1\n",
    ] {
        assert_accepts_valid_starred_expression(
            source,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}
