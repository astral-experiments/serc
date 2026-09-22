use crate::common;

#[test]
fn matches_cpython_marshal_for_conditional_match_comprehension_exit_target() {
    let source = r"def collect(record):
    result = []
    match record['kind']:
        case 'first' | 'second' | 'third':
            if entries := record.get('entries'):
                result = [entry['value'] for entry in entries.values() if entry and entry['value']]
        case 'items':
            for item in record.get('items', []):
                if (value := item.get('value')) is not None:
                    result.append(value)
    return list(map(normalize, result))
";
    common::assert_matches_cpython_marshal(
        source,
        "conditional_match_comprehension_exit_target.py",
    );
}

#[test]
fn matches_cpython_marshal_for_conditional_match_comprehension_exit_store_control() {
    let source = r"def collect(record):
    result = []
    match record['kind']:
        case 'first' | 'second' | 'third':
            if entries := record.get('entries'):
                pending = [entry['value'] for entry in entries.values() if entry and entry['value']]
                result = pending
        case 'items':
            for item in record.get('items', []):
                if (value := item.get('value')) is not None:
                    result.append(value)
    return list(map(normalize, result))
";
    common::assert_matches_cpython_marshal(
        source,
        "conditional_match_comprehension_exit_store_control.py",
    );
}

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
