#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_terminal_with_and_returns() {
    let source = r#"def call_pair(manager, enabled, data):
    if not enabled:
        prepare()
    else:
        with manager:
            value = read(data)
            if first(value) and second(value):
                return False
    return True

def name_call(manager, value):
    with manager:
        reply = load(value)
        if reply and reply.get("success"):
            return reply
    return None

def call_compare(manager, value):
    with manager:
        if ready(value) and read(value) == "done":
            return True
    return False

def compare_pair(manager, value, expected):
    with manager:
        if value is not None and value == expected:
            return True
    return False

def negated_calls(manager, force):
    with manager:
        if not enabled() and not force:
            return False
    return True
"#;
    common::assert_matches_cpython_marshal(source, "with_terminal_and_returns.py");
}
