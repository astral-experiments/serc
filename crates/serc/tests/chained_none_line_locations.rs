use serc::{CompileOptions, compile_bytes_with_options, compile_with_options};

#[path = "common/cpython.rs"]
mod cpython;

fn assert_matches_cpython(source: &str) {
    for version in cpython::VERSIONS {
        let target = version.target();
        let oracle = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
code = compile(sys.stdin.buffer.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
        let filename = "chained_none_line_locations.py";
        let expected = cpython::run_with_input(
            cpython::command(version)
                .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
                .args([
                    target.tag.trim_start_matches('v'),
                    &target.magic_hex(),
                    filename,
                ]),
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
        for actual in [
            compile_with_options(source, filename, &options).unwrap(),
            compile_bytes_with_options(source.as_bytes(), filename, &options).unwrap(),
        ] {
            assert_eq!(actual.marshal(), expected.stdout, "{version:?}: {source}");
        }
    }
}

#[test]
fn specialized_chained_none_tests_preserve_operand_lines() {
    for operator in ["is", "is not"] {
        for comparison in [
            format!("left is right {operator} None"),
            format!("left\n        is right\n        {operator} None"),
            format!("left is right {operator}\n        None"),
        ] {
            for test in [&comparison, &format!("not ({comparison})")] {
                for statement in [
                    format!("assert ({test})"),
                    format!("if ({test}):\n        return left\n    return right"),
                ] {
                    assert_matches_cpython(&format!("def f(left, right):\n    {statement}\n"));
                }
            }
        }
    }
}
