use crate::common;

#[test]
fn matches_cpython_loads_after_direct_manager_try_exits() {
    // These nested manager exits have a separate existing NOP difference. Compare every
    // local-load occurrence directly with the pinned oracle, including all cleanup copies.
    const ORACLE: &str = r#"
import dis, marshal, pathlib, sys, types
source = pathlib.Path(sys.argv[1]).read_text()
ours = marshal.loads(pathlib.Path(sys.argv[2]).read_bytes())
reference = compile(source, "direct_manager_try_exit.py", "exec", dont_inherit=True, optimize=0)
def loads(module):
    code = next(value for value in module.co_consts if isinstance(value, types.CodeType))
    return [(instruction.opname, instruction.arg, instruction.argrepr, tuple(instruction.positions))
            for instruction in dis.get_instructions(code) if "LOAD_FAST" in instruction.opname]
expected = loads(reference)
assert expected, "oracle must contain local loads"
assert loads(ours) == expected, "local-load opcode, argument or location differs"
"#;
    for scope in 0..4 {
        let is_async = scope == 1 || scope == 3;
        let prefix = if is_async { "async " } else { "" };
        let awaited = if is_async { "    await pause()\n" } else { "" };
        let yielded = if scope >= 2 { "    yield value\n" } else { "" };
        for body in [
            "work(value)",
            "if flag:\n    work(value)",
            "if flag:\n    try:\n        work(value)\n    finally:\n        finish(value)",
            "try:\n    work(value)\nfinally:\n    cleanup_a(value)\n    cleanup_b(value)\n    cleanup_c(value)",
        ] {
            for handler in ["raise", "return", "report(value)\nraise"] {
                let source = format!(
                    "{prefix}def function(outer, inner, value, flag, pause):\n{awaited}{yielded}    with outer:\n        try:\n            with inner:\n                {}\n        except Error:\n            {}\n    consume(value)\n{yielded}",
                    body.replace('\n', "\n                "),
                    handler.replace('\n', "\n            "),
                );
                let directory = tempfile::tempdir().unwrap();
                let source_path = directory.path().join("source.py");
                let marshal_path = directory.path().join("ours.marshal");
                std::fs::write(&source_path, &source).unwrap();
                std::fs::write(
                    &marshal_path,
                    serc::compile(&source, "direct_manager_try_exit.py")
                        .unwrap()
                        .marshal(),
                )
                .unwrap();
                let output = common::python_3145_command()
                    .args(["-c", ORACLE])
                    .arg(&source_path)
                    .arg(&marshal_path)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr),
                );
            }
        }
    }
}

#[test]
fn preserves_cpython_marshal_for_intervening_manager_branch_joins() {
    for prefix in ["", "async "] {
        for named in [false, true] {
            for multiple in [false, true] {
                let name = if named { " as error" } else { "" };
                let cause = if named { " from error" } else { "" };
                let additional = if multiple {
                    "            except Exception as error:\n                raise Last(value) from error\n"
                } else {
                    ""
                };
                let source = format!(
                    "{prefix}def function(outer, inner, value, flag):\n    with outer:\n        if flag:\n            try:\n                with inner:\n                    value.work()\n            except Error{name}:\n                raise Other(value){cause}\n{additional}        assert not value.flag\n        try:\n            value.flag = True\n            value.result = value.work()\n        finally:\n            value.flag = False\n    return value\n",
                );
                common::assert_matches_cpython_marshal(&source, "manager_branch_join.py");
            }
        }
    }
}
