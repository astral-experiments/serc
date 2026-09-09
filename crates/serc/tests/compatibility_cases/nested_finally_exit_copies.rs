use crate::common;

#[test]
fn matches_cpython_nested_finally_shared_reraise_exits() {
    for prefix in ["", "async "] {
        for condition in ["flag", "not flag"] {
            for suffix in ["", "        following()\n"] {
                let awaited = if prefix.is_empty() { "" } else { "await " };
                let source = format!(
                    "{prefix}def f(flag):\n    try:\n        {awaited}work()\n    finally:\n        try:\n            {awaited}close()\n        finally:\n            if {condition}:\n                finish()\n{suffix}",
                );
                common::assert_matches_cpython_marshal(&source, "nested_finally_shared_exits.py");
            }
        }
    }
}

#[test]
fn matches_cpython_nested_try_else_shared_reraise_exits() {
    for condition in ["flag", "not flag"] {
        for suffix in ["", "        following()\n"] {
            let source = format!(
                "def f(flag):\n    try:\n        work()\n    finally:\n        try:\n            close()\n        except Error:\n            recover()\n        else:\n            if {condition}:\n                finish()\n{suffix}",
            );
            common::assert_matches_cpython_marshal(&source, "nested_try_else_shared_exits.py");
        }
    }
}
