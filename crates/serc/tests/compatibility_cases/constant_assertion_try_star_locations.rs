use crate::common;

#[test]
fn matches_cpython_shared_exception_group_loop_completion() {
    for (scope, call) in [("", "work()"), ("async ", "await work()")] {
        let source = format!(
            "{scope}def retry(work):\n    attempts = 0\n    while True:\n        try:\n            return {call}\n        except* TimeoutError:\n            if attempts >= 3:\n                raise\n            attempts += 1\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "shared_exception_group_loop_completion.py",
        );
    }
}

#[test]
fn matches_cpython_constant_assertion_try_star_exit_locations() {
    for (scope, prefix) in [("", ""), ("", "    yield None\n"), ("async ", "")] {
        for body_prefix in ["", "            work(value)\n"] {
            for handler_body in ["pass", "assert True"] {
                let source = format!(
                    "{scope}def collect(value):\n{prefix}    try:\n        prepare(value)\n    except Error:\n        try:\n{body_prefix}            assert True\n        except* Other:\n            {handler_body}\n    return value\n"
                );
                common::assert_matches_cpython_marshal(&source, "constant_assertion_try_star.py");
            }
        }
    }
}
