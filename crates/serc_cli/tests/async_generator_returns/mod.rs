use serc::{CompileError, CompileOptions, PythonVersion, compile_with_options};

use crate::common;

fn oracle(version: PythonVersion) -> std::process::Command {
    match version {
        PythonVersion::Py312 => common::python_312_command(),
        PythonVersion::Py313 => common::python_313_command(),
        PythonVersion::Py314 => common::python_3145_command(),
        PythonVersion::Py315 => common::python_315_command(),
    }
}

const VERSIONS: [PythonVersion; 4] = [
    PythonVersion::Py312,
    PythonVersion::Py313,
    PythonVersion::Py314,
    PythonVersion::Py315,
];

#[test]
fn rejects_async_generator_value_returns_from_eager_nested_headers() {
    let header_cases = [
        "async def f():\n    class C((yield 1)): pass\n    return 1\n",
        "async def f():\n    class C(metaclass=(yield 1)): pass\n    return None\n",
        "async def f():\n    @(yield 1)\n    class C: pass\n    return 1\n",
        "async def f():\n    @(yield 1)\n    class C[T]: pass\n    return 1\n",
        "async def f():\n    @(yield 1)\n    def g(): pass\n    return 1\n",
        "async def f():\n    def g(x=(yield 1)): pass\n    return 1\n",
        "async def f():\n    def g[T](x=(yield 1)): pass\n    return 1\n",
        "async def f():\n    async def g(*, x=(yield 1)): pass\n    return 1\n",
        "async def f():\n    g = lambda x=(yield 1): x\n    return 1\n",
        "async def f():\n    return 1\n    class C((yield 1)): pass\n",
        "async def f():\n    if False:\n        class C((yield 1)): pass\n    return 1\n",
        "async def f():\n    class C((yield 1)): pass\n    if False:\n        return 1\n",
        "async def f():\n    class C((yield 1)): pass\n    return *values\n",
        "async def f():\n    class C((yield 1)): pass\n    try: pass\n    except* Exception: return 1\n",
        "async def f():\n    class C((yield 1)): pass\n    return 1\n    break\n",
    ];
    for version in VERSIONS {
        let mut cases = header_cases.to_vec();
        if version < PythonVersion::Py314 {
            cases.extend([
                "async def f():\n    def g(x: (yield 1)): pass\n    return 1\n",
                "async def f():\n    def g() -> (yield 1): pass\n    return 1\n",
            ]);
        }
        for source in cases {
            let native = oracle(version)
                .args([
                    "-c",
                    "import sys; compile(sys.argv[1], 'async_generator_return.py', 'exec')",
                    source,
                ])
                .output()
                .unwrap();
            assert!(!native.status.success(), "{version}: {source}");
            assert!(
                String::from_utf8_lossy(&native.stderr)
                    .contains("'return' with value in async generator"),
                "{version}: {source}: {native:?}"
            );

            let error = compile_with_options(
                source,
                "async_generator_return.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap_err();
            let CompileError::InvalidSyntax { message, range } = error else {
                panic!("{version}: {source}: {error:?}");
            };
            assert_eq!(message, "`return` with value in async generator");
            assert!(source[range].starts_with("return "));
        }
    }
}

#[test]
fn async_generator_return_checks_preserve_independent_scopes_and_bare_returns() {
    let cases = [
        "async def f():\n    class C((yield 1)): pass\n    return\n",
        "async def f():\n    class C((yield 1)):\n        def method(self): return 1\n",
        "async def f():\n    def g(): yield 1\n    return 1\n",
        "async def f():\n    async def g(): yield 1\n    return 1\n",
        "async def f():\n    g = lambda: (yield 1)\n    return 1\n",
        "async def f():\n    class C:\n        def method(self): yield 1\n    return 1\n",
        "async def f():\n    def g(x: (lambda: (yield 1))): pass\n    return 1\n",
        "from __future__ import annotations\nasync def f():\n    def g(x: (lambda: (yield 1))): pass\n    return 1\n",
    ];
    for version in VERSIONS {
        for source in cases {
            let directory = tempfile::tempdir().unwrap();
            let source_path = directory.path().join("async_generator_return.py");
            let pyc_path = directory.path().join("async_generator_return.pyc");
            fs_err::write(&source_path, source).unwrap();
            let native = oracle(version)
                .args([
                    "-c",
                    "import py_compile, sys; py_compile.compile(sys.argv[1], cfile=sys.argv[2], doraise=True)",
                ])
                .arg(&source_path)
                .arg(&pyc_path)
                .output()
                .unwrap();
            assert!(native.status.success(), "{version}: {source}: {native:?}");
            let compiled = crate::serc()
                .args([
                    "compile",
                    "--quiet",
                    "--python-version",
                    &version.to_string(),
                ])
                .arg(&source_path)
                .output()
                .unwrap();
            assert!(
                compiled.status.success(),
                "{version}: {source}: {compiled:?}"
            );
            let (_, minor, _) = version.target().version;
            let serc_pyc = directory.path().join(format!(
                "__pycache__/async_generator_return.cpython-3{minor}.pyc"
            ));
            assert_eq!(
                fs_err::read(serc_pyc).unwrap(),
                fs_err::read(pyc_path).unwrap(),
                "{version}: {source}"
            );
        }
    }
}

#[test]
fn async_generator_return_checks_preserve_prior_scope_errors() {
    for version in VERSIONS {
        let mut cases = vec![
            (
                "async def f():\n    class C((yield 1)): pass\n    break\n    return 1\n",
                "'break' outside loop",
            ),
            (
                "async def f():\n    class C[T]((yield 1)): pass\n    return 1\n",
                "generic",
            ),
            (
                "from __future__ import annotations\nasync def f():\n    def g(x: (yield 1)): pass\n    return 1\n",
                "annotation",
            ),
        ];
        if version >= PythonVersion::Py314 {
            cases.push((
                "async def f():\n    def g(x: (yield 1)): pass\n    return 1\n",
                "annotation",
            ));
        }
        for (source, scope) in cases {
            let native = oracle(version)
                .args([
                    "-c",
                    "import sys; compile(sys.argv[1], 'async_generator_return.py', 'exec')",
                    source,
                ])
                .output()
                .unwrap();
            assert!(!native.status.success(), "{version}: {source}");
            assert!(
                String::from_utf8_lossy(&native.stderr).contains(scope),
                "{version}: {source}: {native:?}"
            );
            let error = compile_with_options(
                source,
                "async_generator_return.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap_err();
            let CompileError::InvalidSyntax { message, .. } = error else {
                panic!("{version}: {source}: {error:?}");
            };
            assert!(message.contains(scope), "{version}: {source}: {message}");
        }
    }
}
