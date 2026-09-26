mod common;

use std::process::Command;

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

#[test]
fn repeated_call_keywords_follow_native_lowering_boundaries() {
    let cases = [
        (
            "*a\nf(x=1, x=2)\n",
            Some("can't use starred expression here"),
        ),
        (
            "f(x=1, x=2)\nclass C(meta=1, meta=2): pass\n",
            Some("keyword argument repeated: x"),
        ),
        ("f(x=1, x=2)\n", Some("keyword argument repeated: x")),
        (
            "f(x=1, **values, x=2)\n",
            Some("keyword argument repeated: x"),
        ),
        (
            "f(x=1, y=1, y=2, x=2)\n",
            Some("keyword argument repeated: x"),
        ),
        ("f(\u{212a}=1, K=2)\n", Some("keyword argument repeated: K")),
        (
            "if False:\n    f(x=1, x=2)\n",
            Some("keyword argument repeated: x"),
        ),
        (
            "def f():\n    return\n    g(x=1, x=2)\n",
            Some("keyword argument repeated: x"),
        ),
        (
            "value = True or f(x=1, x=2)\n",
            Some("keyword argument repeated: x"),
        ),
        (
            "value = lambda: f(x=1, x=2)\n",
            Some("keyword argument repeated: x"),
        ),
        (
            "value = [f(x=1, x=2) for x in ()]\n",
            Some("keyword argument repeated: x"),
        ),
        (
            "@f(x=1, x=2)\ndef g(): pass\n",
            Some("keyword argument repeated: x"),
        ),
        (
            "def g(v=f(x=1, x=2)): pass\n",
            Some("keyword argument repeated: x"),
        ),
        ("value: f(x=1, x=2)\n", Some("keyword argument repeated: x")),
        (
            "f(x=1, x=2)\nx=1\nglobal x\n",
            Some("name 'x' is assigned to before global declaration"),
        ),
        (
            "f(x=1, x=2)\nfrom __future__ import annotations\n",
            Some("from __future__ imports must occur at the beginning of the file"),
        ),
        ("f(x=1, y=2)\n", None),
        ("f(x=1, **{'x': 2})\n", None),
        ("f(**{'x': 1}, **{'x': 2})\n", None),
        (
            "from __future__ import annotations\nvalue: f(x=1, x=2)\n",
            None,
        ),
        (
            "from __future__ import annotations\ndef f(x: g(x=1, x=2)) -> g(x=1, x=2): pass\n",
            None,
        ),
        ("def f():\n    value: g(x=1, x=2)\n", None),
        ("def f():\n    value: (lambda: g(x=1, x=2))\n", None),
        (
            "from __future__ import annotations\nvalue: (lambda: g(x=1, x=2))\n",
            None,
        ),
        (
            "class C:\n    value: f(x=1, x=2)\ng(y=1, y=2)\n",
            Some("keyword argument repeated: x"),
        ),
        (
            "first: f(x=1, x=2)\nsecond: g(y=1, y=2)\n",
            Some("keyword argument repeated: x"),
        ),
        (
            "f(x=1, x=2).attr: g(y=1, y=2)\n",
            Some("keyword argument repeated: x"),
        ),
        (
            "f(x=1, x=2).attr: g(y=1, y=2) = 0\n",
            Some("keyword argument repeated: x"),
        ),
        (
            "f(x=1, x=2)[0]: g(y=1, y=2)\n",
            Some("keyword argument repeated: x"),
        ),
        (
            "f(x=1, x=2).attr: g(y=1, y=2) = h(z=1, z=2)\n",
            Some("keyword argument repeated: z"),
        ),
        (
            "from __future__ import annotations\nvalue: f(x=1, x=2)\ng(y=1, y=2)\n",
            Some("keyword argument repeated: y"),
        ),
    ];
    let annotation_cases = [
        (
            "value: f(x=1, x=2)\ng(y=1, y=2)\n",
            Some("keyword argument repeated: x"),
            Some("keyword argument repeated: y"),
        ),
        (
            "class C:\n    value: f(x=1, x=2)\n    g(y=1, y=2)\n",
            Some("keyword argument repeated: x"),
            Some("keyword argument repeated: y"),
        ),
        (
            "if flag:\n    value: f(x=1, x=2)\n    g(y=1, y=2)\n",
            Some("keyword argument repeated: x"),
            Some("keyword argument repeated: y"),
        ),
        (
            "value: f(x=1, x=2)\nclass C:\n    g(y=1, y=2)\n",
            Some("keyword argument repeated: x"),
            Some("keyword argument repeated: y"),
        ),
        (
            "obj.attr: f(x=1, x=2)\n",
            Some("keyword argument repeated: x"),
            None,
        ),
    ];
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("repeated_keywords.py");
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
        let modern = matches!(version, PythonVersion::Py314 | PythonVersion::Py315);
        let annotations = annotation_cases
            .iter()
            .map(|&(source, legacy, deferred)| (source, if modern { deferred } else { legacy }));
        for (source, mut expected) in cases.into_iter().chain(annotations) {
            if source.starts_with("f(x=1, x=2)\nfrom __future__")
                && matches!(version, PythonVersion::Py312 | PythonVersion::Py313)
            {
                expected = Some("keyword argument repeated: x");
            }
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
                    let expected = serc_message(expected);
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
                    String::from_utf8_lossy(&output.stderr).contains(serc_message(expected)),
                    "{output:?}"
                );
            }
        }
    }
}

fn serc_message(native: &str) -> &str {
    if native == "from __future__ imports must occur at the beginning of the file" {
        "__future__ imports must be at the top of the file"
    } else {
        native
    }
}
