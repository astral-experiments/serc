use crate::common;

#[test]
fn matches_cpython_nested_if_while_original_join_locations() {
    let source = r#"def dispatch(read):
    while True:
        command = read()
        if command == "stop":
            break
        if command == "skip":
            visit()
        elif command == "write":
            result = prepare()
            if result:
                first, second = result
                publish(first, second)
        elif command == "later":
            finish()
        else:
            unknown()
"#;
    common::assert_matches_cpython_marshal(source, "nested_if_while_dispatch.py");
}
