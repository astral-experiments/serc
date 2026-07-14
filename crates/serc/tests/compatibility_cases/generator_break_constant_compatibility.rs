use crate::common;

#[test]
fn matches_cpython_marshal_for_terminal_generator_break_constants() {
    let source = r#"def direct_break(flag):
    while True:
        if not flag:
            break
        value = "lineend"
        yield value

def leading_assignment(read, encode):
    while True:
        value = read(4)
        if not value:
            break
        if encode:
            value = value.encode("utf-8")
        yield value

def leading_annotated_assignment(read):
    while True:
        value: bytes = read(4)
        if not value:
            break
        elif len(value) != 4:
            raise ValueError("short read")
        yield value
"#;
    common::assert_matches_cpython_marshal(source, "generator_break_constant_compatibility.py");
}
