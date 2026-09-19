use crate::common;

#[test]
fn matches_cpython_nonlocal_comprehension_slots_after_writes() {
    for prefix in ["", "async "] {
        for write in ["", "value = make()\n", "value += 1\n", "del value\n"] {
            for expression in [
                "[value for value in values]",
                "{value: item for value, item in values}",
            ] {
                for helper in [false, true] {
                    let body = format!(
                        "nonlocal value\n{write}result = {expression}\nreturn {}value, result",
                        if helper { "helper(), " } else { "" },
                    );
                    let inner = format!(
                        "{prefix}def inner(values):\n    {}",
                        body.replace('\n', "\n    "),
                    );
                    let source = format!(
                        "def outer(value):\n{}    {}\n    return inner\n",
                        if helper {
                            "    def helper():\n        return value\n"
                        } else {
                            ""
                        },
                        inner.replace('\n', "\n    "),
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "nonlocal_comprehension_shadow.py",
                    );
                }
            }
        }
    }
}
