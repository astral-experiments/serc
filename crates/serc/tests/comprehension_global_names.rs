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
        let filename = "comprehension_global_names.py";
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
fn temporary_targets_do_not_bind_independent_global_references() {
    for reference in [
        "result = [operation(value) for value in values]",
        "result = {operation(value) for value in values}",
        "result = {value: operation(value) for value in values}",
        "result = (operation(value) for value in values)",
        "result = lambda: operation()",
        "def result():\n    return operation()",
        "class result:\n    def method(self):\n        return operation()",
    ] {
        for reference_first in [false, true] {
            let temporary = "first = [operation for operation in values]";
            let body = if reference_first {
                format!("{reference}\n{temporary}\nreturn result, first")
            } else {
                format!("{temporary}\n{reference}\nreturn result, first")
            };
            let function = format!(
                "def function(values):\n{}\n",
                body.lines()
                    .map(|line| format!("    {line}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            assert_matches_cpython(&function);
            for parameters in ["", "operation"] {
                let mut source = function.clone();
                for depth in 0..2 {
                    source = format!(
                        "def outer_{depth}({parameters}):\n{}\n",
                        source
                            .lines()
                            .map(|line| format!("    {line}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    );
                    assert_matches_cpython(&source);
                }
            }
        }
    }
}
