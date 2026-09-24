use serc::{
    CompileError, CompileOptions, PythonVersion, compile, compile_bytes, compile_with_options,
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
