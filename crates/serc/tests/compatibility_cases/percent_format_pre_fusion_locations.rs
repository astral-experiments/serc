use crate::common;

#[test]
fn matches_cpython_percent_prefix_at_try_entry() {
    for body in [
        "return \"prefix %s-%s\" % (value, value)",
        "return \"prefix %r\" % (value,)",
        "return \"%s-%s\" % (value, value)",
        "return \"prefix %s\" % value",
        "pass\n        return \"prefix %s-%s\" % (value, value)",
    ] {
        let source = format!(
            "def f(value):\n    try:\n        {body}\n    except Exception:\n        return value\n"
        );
        common::assert_matches_cpython_marshal(&source, "percent_prefix_try_entry.py");
    }
    for source in [
        r#"def f(value, flag):
    if flag:
        try:
            return "prefix %s-%s" % (value, value)
        except Exception:
            return value
    return None
"#,
        r#"def f(value):
    try:
        result = "prefix %s-%s" % (value, value)
    finally:
        finish()
    return result
"#,
        r#"def f(value):
    with manager():
        try:
            return "prefix %s-%s" % (value, value)
        except Exception:
            return value
"#,
        r#"async def f(value):
    try:
        return "prefix %s-%s" % (value, value)
    except Exception:
        return value
"#,
        r#"def f(value):
    yield value
    try:
        yield "prefix %s-%s" % (value, value)
    except Exception:
        return value
"#,
    ] {
        common::assert_matches_cpython_marshal(source, "percent_prefix_try_contexts.py");
    }
}

#[test]
fn removes_literal_marker_after_percent_prefix_inherits_its_line() {
    for function in ["def", "async def"] {
        for marker in ["pass", "\"unused statement\"", "42", "1 + 2"] {
            let source = format!(
                "{function} f(value):\n    anchor = value\n    {marker}\n    result = \"prefix %s\" % (value,)\n    return result\n"
            );
            common::assert_matches_cpython_marshal(&source, "percent_prefix_literal_donor.py");
        }
    }
}
