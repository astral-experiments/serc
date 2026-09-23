use std::process::Command;

use serc::{CompileError, CompileOptions, PythonVersion, compile_with_options};

use crate::common;

fn python(version: PythonVersion) -> Command {
    match version {
        PythonVersion::Py312 => common::python_312_command(),
        PythonVersion::Py313 => common::python_313_command(),
        PythonVersion::Py314 => common::python_3145_command(),
        PythonVersion::Py315 => common::python_315_command(),
    }
}

#[test]
fn rejects_transfers_leaving_except_star_before_discarding_code() {
    let mut sources = vec![
        "def f():\n    try: pass\n    except* Exception: return\n".to_owned(),
        "def f():\n    try: pass\n    except* Exception: return value\n".to_owned(),
        "def f():\n    return\n    try: pass\n    except* Exception: return\n".to_owned(),
        "def f():\n    try: pass\n    except* Exception:\n        if False: return\n".to_owned(),
        "def f():\n    try: pass\n    except* Exception:\n        try: pass\n        except Exception: return\n".to_owned(),
        "def f():\n    try: pass\n    except* Exception:\n        try: pass\n        finally: return\n".to_owned(),
        "def f():\n    try: pass\n    except* Exception:\n        with manager: return\n".to_owned(),
    ];
    for transfer in ["break", "continue"] {
        sources.extend([
            format!("try: pass\nexcept* Exception: {transfer}\n"),
            format!("while condition:\n    try: pass\n    except* Exception: {transfer}\n"),
            format!("while False:\n    try: pass\n    except* Exception: {transfer}\n"),
            format!("while condition:\n    try: pass\n    except* Exception:\n        for x in xs: pass\n        else: {transfer}\n"),
            format!("try: pass\nexcept* Exception:\n    while condition:\n        try: pass\n        except* Exception: {transfer}\n"),
        ]);
    }
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for source in &sources {
            let native = python(version)
                .args([
                    "-c",
                    "import sys; compile(sys.argv[1], 'except_star.py', 'exec')",
                    source,
                ])
                .output()
                .unwrap();
            let message = "'break', 'continue' and 'return' cannot appear in an except* block";
            assert!(!native.status.success(), "{version}: {source}");
            assert!(
                String::from_utf8_lossy(&native.stderr)
                    .contains(&format!("SyntaxError: {message}")),
                "{version}: {source}: {native:?}"
            );
            let error = compile_with_options(
                source,
                "except_star.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap_err();
            let CompileError::InvalidSyntax {
                message: actual,
                range,
            } = error
            else {
                panic!("{version}: {source}: {error:?}");
            };
            assert_eq!(actual, message, "{version}: {source}");
            assert!(
                ["break", "continue", "return"]
                    .iter()
                    .any(|keyword| source[range].starts_with(keyword)),
                "{version}: {source}: {range:?}"
            );
        }
    }
}

#[test]
fn except_star_boundaries_preserve_valid_code_and_nested_scope_diagnostics() {
    let mut sources = vec![
        "def f():\n    try: return\n    except* Exception: pass\n".to_owned(),
        "def f():\n    try: pass\n    except* Exception: pass\n    else: return\n".to_owned(),
        "def f():\n    try: pass\n    except* Exception: pass\n    finally: return\n".to_owned(),
        "def f():\n    try: pass\n    except* Exception: pass\n    return\n".to_owned(),
        "def f():\n    try: pass\n    except Exception: return\n".to_owned(),
        "try: pass\nexcept* Exception:\n    def f(): return value\n    async def g(): return value\n    class C:\n        def method(self): return value\n    value = lambda: result\n".to_owned(),
        "try: pass\nexcept* Exception:\n    @decorate(lambda: value)\n    def f(x=(lambda: value)()): return x\n    class C((lambda: object)()): pass\n".to_owned(),
        "async def f():\n    try: pass\n    except* Exception:\n        async for x in xs:\n            if x: break\n            continue\n".to_owned(),
    ];
    for transfer in ["break", "continue"] {
        sources.extend([
            format!("try: pass\nexcept* Exception:\n    while condition: {transfer}\n"),
            format!("try: pass\nexcept* Exception:\n    for x in xs: {transfer}\n"),
            format!("try: pass\nexcept* Exception:\n    class C:\n        while condition: {transfer}\n"),
            format!("try: pass\nexcept* Exception:\n    while condition:\n        try: pass\n        except* Exception: pass\n        {transfer}\n"),
            format!("try: pass\nexcept* Exception:\n    while condition:\n        for x in xs: pass\n        else: {transfer}\n"),
            format!("while condition:\n    try: pass\n    except* Exception: pass\n    {transfer}\n"),
            format!("while condition:\n    try: pass\n    except Exception: {transfer}\n"),
        ]);
    }
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
        for source in &sources {
            let native = python(version)
                .args([
                    "-c",
                    "import sys; compile(sys.argv[1], 'except_star.py', 'exec')",
                    source,
                ])
                .output()
                .unwrap();
            assert!(native.status.success(), "{version}: {source}: {native:?}");
            compile_with_options(source, "except_star.py", &options)
                .unwrap_or_else(|error| panic!("{version}: {source}: {error:?}"));
        }
        for (source, message) in [
            (
                "try: pass\nexcept* Exception: return\n",
                "'return' outside function",
            ),
            (
                "def f():\n    try: pass\n    except* Exception:\n        class C: return\n",
                "'return' outside function",
            ),
            (
                "while condition:\n    try: pass\n    except* Exception:\n        def f(): break\n",
                "'break' outside loop",
            ),
            (
                "while condition:\n    try: pass\n    except* Exception:\n        class C: continue\n",
                "'continue' not properly in loop",
            ),
        ] {
            let native = python(version)
                .args([
                    "-c",
                    "import sys; compile(sys.argv[1], 'except_star.py', 'exec')",
                    source,
                ])
                .output()
                .unwrap();
            assert!(!native.status.success(), "{version}: {source}");
            assert!(
                String::from_utf8_lossy(&native.stderr)
                    .contains(&format!("SyntaxError: {message}"))
            );
            let error = compile_with_options(source, "except_star.py", &options).unwrap_err();
            assert!(
                matches!(error, CompileError::InvalidSyntax { message: actual, .. } if actual == message)
            );
        }
    }
}

#[test]
fn invalid_return_expressions_precede_except_star_transfer_errors() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (expression, native_message, expected_message) in [
            (
                "*values",
                "can't use starred expression here",
                "Starred expression cannot be used here",
            ),
            (
                "await value",
                "'await' outside async function",
                "'await' outside function",
            ),
        ] {
            let source =
                format!("def f():\n    try: pass\n    except* Exception: return {expression}\n");
            let native = python(version)
                .args([
                    "-c",
                    "import sys; compile(sys.argv[1], 'except_star.py', 'exec')",
                    &source,
                ])
                .output()
                .unwrap();
            assert!(!native.status.success(), "{version}: {source}");
            assert!(
                String::from_utf8_lossy(&native.stderr)
                    .contains(&format!("SyntaxError: {native_message}"))
            );
            let error = compile_with_options(
                &source,
                "except_star.py",
                &CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                },
            )
            .unwrap_err();
            assert!(
                matches!(error, CompileError::InvalidSyntax { message, .. } if message == expected_message)
            );
        }
    }
}
