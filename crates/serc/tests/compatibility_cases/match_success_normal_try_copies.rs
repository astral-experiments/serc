use crate::common;

#[test]
fn matches_cpython_match_call_arms_before_literal_return() {
    let source = r"def choose(value):
    try:
        match value:
            case 'first':
                first()
            case 'second':
                second()
    except Error:
        return 1
    return 0
";
    common::assert_matches_cpython_marshal(source, "match_call_literal_return.py");
}

#[test]
fn matches_cpython_match_call_arms_with_named_handler() {
    let source = r"def choose(options):
    try:
        value, extra = prepare(options)
        match options.mode:
            case 'first':
                first(value, extra)
            case 'second':
                second(value)
            case 'third':
                third(value, extra)
    except Exception as error:
        report(error)
        return 1
    return 0
";
    common::assert_matches_cpython_marshal(source, "match_call_named_handler.py");
}

#[test]
fn matches_cpython_match_call_following_call_control() {
    let source = r"def choose(value):
    try:
        match value:
            case 'first':
                first()
            case 'second':
                second()
    except Error:
        return 1
    finish()
    return 0
";
    common::assert_matches_cpython_marshal(source, "match_call_following_call_control.py");
}

#[test]
fn matches_cpython_match_terminating_arms_control() {
    let source = r"def choose(value):
    try:
        match value:
            case 'first':
                return first(value)
            case 'second':
                return second(value)
    except Error:
        return recover(value)
    raise Failure(value)
";
    common::assert_matches_cpython_marshal(source, "match_terminating_arms_control.py");
}

#[test]
fn matches_cpython_match_without_try_control() {
    let source = r"def choose(value):
    match value:
        case 'first':
            first()
        case 'second':
            second()
    return 0
";
    common::assert_matches_cpython_marshal(source, "match_without_try_control.py");
}
