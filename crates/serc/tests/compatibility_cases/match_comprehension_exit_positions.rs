use crate::common;

#[test]
fn matches_cpython_final_refutable_match_comprehension_exit() {
    const SOURCE: &str = r#"def f(mode, items, flag, other):
    match mode:
        case "first":
            value = items
            if flag:
                work(value)
        case "second" | "third":
            if flag and other:
                value = COMPREHENSION
        case _:
            raise Error(mode)
    result = after()
    return value, result
"#;

    for (filename, expression) in [
        ("final_list.py", "[item for item in items if item]"),
        ("final_set.py", "{item for item in items if item}"),
        ("final_dict.py", "{item: item for item in items if item}"),
        ("final_unfiltered.py", "[item for item in items]"),
    ] {
        common::assert_matches_cpython_marshal(
            &SOURCE.replace("COMPREHENSION", expression),
            filename,
        );
    }
}
