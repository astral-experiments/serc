use crate::common;

#[test]
fn matches_cpython_transitive_chained_conditional_returns() {
    for (prefix, suspension) in [("", ""), ("async ", ""), ("", "    yield value\n")] {
        for condition in [
            "a < b < c",
            "not a < b < c",
            "ready or a < b < c",
            "not (ready and a < b < c)",
        ] {
            for value in [
                "value",
                "None",
                "value + 1",
                "value.attr",
                "value.method()",
                "work(value)",
            ] {
                for fallback in ["None", "fallback"] {
                    let source = format!(
                        "{prefix}def f(a, b, c, ready, value, fallback):\n{suspension}    return {value} if {condition} else {fallback}\n",
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "transitive_chained_conditional_returns.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_chained_conditional_return_copy_limits() {
    for condition in ["not a < b < c", "not (ready and a < b < c)"] {
        for value in [
            "value",
            "None",
            "value + 1",
            "value.attr",
            "value.method()",
            "work(value)",
        ] {
            let source = format!(
                "def f(a, b, c, ready, value, fallback):\n    saved = {value} if {condition} else fallback\n    return saved\n",
            );
            common::assert_matches_cpython_marshal(&source, "chained_conditional_return_limits.py");
        }
        for value in ["value.attr", "value.method()", "work(value)"] {
            let source = format!(
                "def f(a, b, c, ready, value, fallback):\n    try:\n        return {value} if {condition} else fallback\n    except Error:\n        report()\n",
            );
            common::assert_matches_cpython_marshal(
                &source,
                "protected_chained_conditional_return_limits.py",
            );
        }
    }
}
