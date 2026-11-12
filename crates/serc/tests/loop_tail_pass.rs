use serc::{CompileOptions, compile_bytes_with_options};

#[path = "common/cpython.rs"]
mod cpython;

fn assert_matches_cpython(source: &str) {
    const ORACLE: &str = "import marshal, sys; code = compile(sys.stdin.buffer.read(), sys.argv[1], 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
    let filename = "loop_tail_pass.py";
    for version in cpython::VERSIONS {
        let expected = cpython::run_with_input(
            cpython::command(version)
                .args(["-B", "-I", "-S", "-X", "utf8", "-c", ORACLE, filename]),
            source.as_bytes(),
        );
        assert!(
            expected.status.success(),
            "{version:?}: {}",
            String::from_utf8_lossy(&expected.stderr)
        );
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        let actual = compile_bytes_with_options(source.as_bytes(), filename, &options).unwrap();
        assert_eq!(actual.marshal(), expected.stdout, "{version:?}: {source}");
    }
}

#[test]
fn protected_loop_tail_pass_inside_an_outer_manager() {
    assert_matches_cpython(
        "def f(items):\n    with manager():\n        for item in items:\n            try:\n                if item:\n                    use(item)\n                    pass\n            except Exception:\n                recover(item)\n",
    );
}

#[test]
fn loop_tail_pass_manager_and_handler_boundaries() {
    for source in [
        "def f(items):\n    with manager():\n        for item in items:\n            if item:\n                use(item)\n                pass\n",
        "def f(items):\n    with manager():\n        for item in items:\n            pass\n",
        "def f(items):\n    for item in items:\n        with manager():\n            if item:\n                use(item)\n                pass\n",
        "def f(items):\n    for item in items:\n        try:\n            if item:\n                use(item)\n                pass\n        except Exception:\n            recover(item)\n",
    ] {
        assert_matches_cpython(source);
    }
}
