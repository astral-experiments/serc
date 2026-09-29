use crate::common;

#[test]
fn matches_cpython_nested_try_branch_exit_markers() {
    for action in ["return read()", "consume(flag)", "value = read()"] {
        for handler in ["pass", "consume(flag)", "return flag"] {
            for successor in ["return None", "return flag", "raise Error"] {
                let branch = format!(
                    "if flag:\n    try:\n        {action}\n    except InnerError:\n        {handler}"
                );
                for in_else in [false, true] {
                    let body = if in_else {
                        format!(
                            "try:\n    consume(flag)\nexcept OuterError:\n    consume(flag)\nelse:\n    {}",
                            branch.replace('\n', "\n    "),
                        )
                    } else {
                        format!(
                            "try:\n    {}\nexcept OuterError:\n    consume(flag)",
                            branch.replace('\n', "\n    "),
                        )
                    };
                    let source = format!(
                        "def f(flag):\n    {}\n    {successor}\n",
                        body.replace('\n', "\n    "),
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "nested_try_branch_exit_markers.py",
                    );
                }
            }
        }
    }
}
