use std::fmt::Write as _;

use serc::{CompileOptions, PythonVersion, compile_with_options};

#[path = "common/cpython.rs"]
mod cpython;

fn assert_matches_cpython(source: &str, version: PythonVersion) {
    let target = version.target();
    let oracle = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
code = compile(sys.stdin.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
    let filename = "exception_group_stack_depth.py";
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
    assert_eq!(
        compile_with_options(source, filename, &options)
            .unwrap()
            .marshal(),
        expected.stdout,
        "{version:?}: {source}"
    );
}

fn indent(source: &str) -> String {
    let mut indented = String::new();
    for line in source.lines() {
        writeln!(indented, "    {line}").unwrap();
    }
    indented
}

#[test]
fn later_group_handlers_retain_the_complete_subgroup_stack() {
    for version in cpython::VERSIONS {
        for preceding in 0..=2 {
            for name in ["", " as error"] {
                for earlier_body in ["pass", "raise"] {
                    for body in [
                        "await work(value)",
                        "await work(value, 'first' if flag else 'second')",
                        "async for item in values:\n    await work(item)",
                        "work(value, value, value, value, value, value, value, value)",
                    ] {
                        let mut suite = String::from("try:\n    await work(value)\n");
                        for index in 0..preceding {
                            writeln!(suite, "except* First{index}:\n    {earlier_body}").unwrap();
                        }
                        write!(suite, "except* Error{name}:\n{}", indent(body)).unwrap();
                        for suite in [
                            suite.clone(),
                            format!("with manager():\n{}", indent(&suite)),
                        ] {
                            let source =
                                format!("async def f(value, flag, values):\n{}", indent(&suite));
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}
