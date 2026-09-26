mod common;

use std::process::Command;

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

#[test]
fn reserved_debug_writes_match_target_rules() {
    let cases = [
        ("obj.__debug__ = 1\n", "store"),
        ("obj.__debug__ += 1\n", "modern"),
        ("obj.__debug__: int\n", "store"),
        ("obj.child.__debug__ = 1\n", "store"),
        ("obj.__debug__, other = values\n", "store"),
        ("for obj.__debug__ in values: pass\n", "store"),
        ("with manager() as obj.__debug__: pass\n", "store"),
        ("if False:\n    obj.__debug__ = 1\n", "store"),
        ("def f():\n    return\n    obj.__debug__ = 1\n", "store"),
        ("f(__debug__=1)\n", "store"),
        ("f(__debug__=1, x=1, x=2)\n", "store"),
        ("f(__debug__=1, __debug__=2)\n", "store"),
        ("f(x=1, x=2, __debug__=1)\n", "duplicate-first"),
        ("f(g(x=1, x=2), __debug__=1)\n", "store"),
        ("g(x=1, x=2)(__debug__=1)\n", "store"),
        ("f(x=g(y=1, y=2), __debug__=1)\n", "store"),
        ("class C(__debug__=1, x=1, x=2): pass\n", "store"),
        ("class C(x=1, x=2, __debug__=1): pass\n", "duplicate-first"),
        (
            "@f(x=1, x=2)\nclass C(__debug__=1): pass\n",
            "duplicate-first",
        ),
        ("class C(__debug__=1):\n    g(y=1, y=2)\n", "body-duplicate"),
        (
            "class C[T](__debug__=1):\n    g(y=1, y=2)\n",
            "body-duplicate",
        ),
        ("class C(__debug__=1):\n    break\n", "body-break"),
        ("class C(g(y=1, y=2), __debug__=1): pass\n", "store"),
        ("f(**values, __debug__=1)\n", "store"),
        ("class C(__debug__=1): pass\n", "store"),
        ("class C[T](__debug__=1): pass\n", "store"),
        ("match obj:\n    case C(__debug__=value): pass\n", "store"),
        ("def f(value=g(__debug__=1)): pass\n", "store"),
        ("value = lambda: f(__debug__=1)\n", "store"),
        ("del obj.__debug__\n", "delete"),
        (
            "from __future__ import annotations\nvalue: f(__debug__=1)\n",
            "modern",
        ),
        ("def f():\n    value: g(__debug__=1)\n", "modern"),
        (
            "from __future__ import annotations\nvalue: (lambda: f(__debug__=1))\n",
            "modern",
        ),
        ("result = obj.__debug__\n", "ok"),
        ("obj.__debug__.value = 1\n", "ok"),
        ("obj.__debug__[0] = 1\n", "ok"),
        ("setattr(obj, '__debug__', 1)\n", "ok"),
        ("f(**{'__debug__': 1})\n", "ok"),
        ("class C(**{'__debug__': 1}): pass\n", "ok"),
    ];
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reserved_debug.py");
    let oracle = r"
import marshal, sys
try:
    code = compile(sys.argv[1], sys.argv[2], 'exec', dont_inherit=True, optimize=0)
except SyntaxError as error:
    print(error.msg)
    raise SystemExit(1)
sys.stdout.buffer.write(marshal.dumps(code))
";
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for (source, kind) in cases {
            let modern = matches!(version, PythonVersion::Py314 | PythonVersion::Py315);
            let expected = match kind {
                "store" => Some("cannot assign to __debug__"),
                "modern" | "duplicate-first" | "body-duplicate" | "body-break" if modern => {
                    Some("cannot assign to __debug__")
                }
                "duplicate-first" => Some("keyword argument repeated: x"),
                "body-duplicate" => Some("keyword argument repeated: y"),
                "body-break" => Some("'break' outside loop"),
                "delete" if modern => Some("cannot delete __debug__"),
                _ => None,
            };
            let mut python = match version {
                PythonVersion::Py312 => common::python_312_command(),
                PythonVersion::Py313 => common::python_313_command(),
                PythonVersion::Py314 => common::python_3145_command(),
                PythonVersion::Py315 => common::python_315_command(),
            };
            let native = python
                .args(["-c", oracle, source])
                .arg(&path)
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
            fs_err::write(&path, source).unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_serc"))
                .args([
                    "compile",
                    "--check",
                    "--quiet",
                    "--python-version",
                    &version.to_string(),
                ])
                .arg(&path)
                .output()
                .unwrap();
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
    }
}
