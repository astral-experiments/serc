use crate::common;

#[test]
fn preserves_existing_nested_exit_1() {
    common::assert_matches_cpython_marshal(
        r#""""Public structural reduction candidate; not compiled or executed yet.

Exercise a terminal conditional following try/except inside nested if/elif
arms, with the return emitted after the outer chain.
"""
import json


def validate_payload(state, text, errors):
    if state == "empty":
        if text:
            errors.append("unexpected text")
    elif state in ("partial", "complete"):
        if state == "complete":
            if not isinstance(text, str) or not text.strip():
                errors.append("missing text")
            else:
                try:
                    decoded = json.loads(text)
                except json.JSONDecodeError:
                    decoded = None
                if not isinstance(decoded, dict):
                    errors.append("expected an object")
        elif text not in ("", None):
            errors.append("unexpected partial text")
    elif state in ("invalid", "mixed"):
        if text not in ("", None):
            errors.append("unexpected invalid text")
    return state
"#,
        "nested_decode_join.py",
    );
}

#[test]
fn preserves_existing_nested_exit_2() {
    common::assert_matches_cpython_marshal(
        r#"def nested_for_return(outer, inner, guard, values, sink, result):
    if outer:
        if inner:
            sink(result)
        else:
            if guard:
                for value in values:
                    if value not in result:
                        sink(value)
    elif guard:
        sink(result)
    return result


def event_shape(event, issues, sink):
    kind = event.get("kind")
    if kind == "request":
        if event.get("accepted"):
            if event.get("served"):
                sink(issues)
            else:
                if not event.get("first"):
                    sink(issues)
                if event.get("first") and event.get("first") == event.get("last"):
                    for field in ("first", "last"):
                        if field not in issues:
                            issues.append(field)
        elif event.get("requested"):
            sink(issues)
        elif event.get("observed"):
            if "context" not in issues:
                issues.append("context")
    elif kind == "attempt":
        sink(issues)
    return issues


def for_completion_control(values, sink, result):
    for value in values:
        if value not in result:
            sink(value)
    return result
"#,
        "nested_if_for_copied_return.py",
    );
}
