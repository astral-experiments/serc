use crate::common;

// The earlier nested arms still have a separate small-exit copying difference.
// Compare every NOP position and the results of all branches without normalizing bytecode.
#[test]
fn nested_match_exit_respects_the_small_copy_limit() {
    let tuple_source = r#"def f(subject, a, b, c):
    value = 0
    tag = "initial"
    match subject:
        case None:
            if a:
                if b:
                    value = 1
                    tag = "one"
                elif c:
                    value = 2
                    tag = "two"
                else:
                    value = 3
                    tag = "three"
            elif b:
                value = 4
                tag = "middle"
            else:
                value = 5
                tag = "last"
        case "next":
            if b:
                value = 6
                tag = "yes"
            else:
                value = 7
                tag = "no"
        case _:
            raise ValueError
    return value, tag
"#;
    let name_source = tuple_source.replace("return value, tag\n", "return value\n");
    for (filename, source) in [
        ("nested_match_tuple_return.py", tuple_source),
        ("nested_match_name_return.py", name_source.as_str()),
    ] {
        let inspector = r#"
import dis, marshal, sys
if sys.argv[1] == "source":
    code = compile(sys.stdin.buffer.read(), sys.argv[2], "exec", dont_inherit=True, optimize=0)
else:
    code = marshal.loads(sys.stdin.buffer.read())
namespace = {}
exec(code, namespace)
f = namespace["f"]
print([
    f(subject, a, b, c)
    for subject in (None, "next")
    for a in (False, True)
    for b in (False, True)
    for c in (False, True)
])
print([
    (
        instruction.positions.lineno,
        instruction.positions.end_lineno,
        instruction.positions.col_offset,
        instruction.positions.end_col_offset,
    )
    for instruction in dis.get_instructions(f)
    if instruction.opname == "NOP"
])
"#;
        let inspect = |mode: &str, input: &[u8]| {
            let output = crate::common::cpython::run_with_input(
                common::python_3145_command().args(["-c", inspector, mode, filename]),
                input,
            );
            assert!(
                output.status.success(),
                "failed to inspect {mode} for {filename}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            output.stdout
        };
        let expected = inspect("source", source.as_bytes());
        let marshal = serc::compile(source, filename).unwrap().marshal();
        let actual = inspect("marshal", &marshal);
        assert_eq!(
            actual, expected,
            "branch values or NOP positions differ for {filename}"
        );
    }
}
