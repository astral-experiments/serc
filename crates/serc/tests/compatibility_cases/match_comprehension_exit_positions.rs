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
