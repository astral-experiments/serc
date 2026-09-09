use std::fmt::Write as _;

use crate::common;

#[test]
fn retained_finally_loop_nops_survive_borrow_analysis() {
    for (generator, locals) in [(false, 0), (true, 0), (false, 70)] {
        let mut source = String::from("def f(cm, flag, value, other, items):\n");
        if generator {
            source.push_str("    yield value\n");
        }
        for index in 0..locals {
            writeln!(source, "    local_{index} = value").unwrap();
        }
        source.push_str(
            "    with cm:\n        try:\n            work(value)\n        finally:\n            for item in items:\n                if True: pass\n",
        );
        common::assert_matches_cpython_marshal(&source, "retained_finally_loop_nop.py");
    }
}

#[test]
fn unlocated_finally_joins_remain_empty_after_borrow_analysis() {
    for generator in [false, true] {
        for tail in [
            "if other:\n                    finish()",
            "for item in items:\n                    finish(item)",
        ] {
            let suspension = if generator { "    yield flag\n" } else { "" };
            let source = format!(
                "def f(cm, flag, other, items):\n{suspension}    with cm:\n        try:\n            work()\n        finally:\n            if flag:\n                work()\n                {tail}\n",
            );
            common::assert_matches_cpython_marshal(&source, "unlocated_finally_join.py");
        }
    }
}

#[test]
fn false_finally_branch_skips_the_authored_pass_line() {
    // The authored pass NOP is still missing in this shape. Its line cannot be moved to
    // the shared join: that would make the false branch execute the true branch's event.
    const SOURCE: &str = "def f(cm, flag, value, other, items):\n    with cm:\n        try:\n            work(value)\n        finally:\n            if flag: pass\n";
    const ORACLE: &str = r#"
import dis, marshal, pathlib, sys, types
ours = marshal.loads(pathlib.Path(sys.argv[1]).read_bytes())
reference = compile(pathlib.Path(sys.argv[2]).read_text(), "finally_pass_branch.py", "exec", dont_inherit=True, optimize=0)
def destination(module):
    code = next(value for value in module.co_consts if isinstance(value, types.CodeType))
    instructions = list(dis.get_instructions(code))
    branch = next(item for item in instructions if item.opname == "POP_JUMP_IF_FALSE" and item.positions.lineno == 6)
    target = next(item for item in instructions if item.offset == branch.argval)
    return target.opname, target.argval, target.positions
assert destination(ours) == destination(reference), "false branch enters the authored pass line"
"#;
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.py");
    let marshal_path = directory.path().join("ours.marshal");
    std::fs::write(&source_path, SOURCE).unwrap();
    std::fs::write(
        &marshal_path,
        serc::compile(SOURCE, "finally_pass_branch.py")
            .unwrap()
            .marshal(),
    )
    .unwrap();
    let output = common::python_3145_command()
        .args(["-c", ORACLE])
        .arg(&marshal_path)
        .arg(&source_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
