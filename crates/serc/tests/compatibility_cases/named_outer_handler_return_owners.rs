use crate::common;
use crate::common::indent;

#[test]
fn matches_cpython_named_outer_handler_return_owners() {
    for (scope, suspension) in [
        ("def", ""),
        ("async def", "await pause()\n"),
        ("def", "yield value\n"),
        ("async def", "yield value\n"),
    ] {
        for returned in ["", " value", " outer"] {
            if scope == "async def" && suspension == "yield value\n" && !returned.is_empty() {
                continue;
            }
            for prefix in ["", "record()\n"] {
                for following in ["", "finish()\n"] {
                    let handler = format!("{prefix}return{returned}\n");
                    let body = format!(
                        "{suspension}try:\n    work()\nexcept Error as outer:\n    try:\n        retry()\n    except Other:\n{}{following}",
                        indent(&indent(&handler)),
                    );
                    let source = format!("{scope} f(value):\n{}", indent(&body));
                    common::assert_matches_cpython_marshal(
                        &source,
                        "named_outer_handler_return.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_named_outer_handler_conditional_return_owners() {
    for (scope, suspension) in [
        ("def", ""),
        ("async def", "await pause()\n"),
        ("def", "yield value\n"),
    ] {
        for returned in [" value", " outer"] {
            for prefix in ["", "record()\n"] {
                let handler = format!("{prefix}if enabled:\n    return{returned}\nfallback()\n");
                let body = format!(
                    "{suspension}try:\n    work()\nexcept Error as outer:\n    try:\n        retry()\n    except Other:\n{}",
                    indent(&indent(&handler)),
                );
                let source = format!("{scope} f(value, enabled):\n{}", indent(&body));
                common::assert_matches_cpython_marshal(
                    &source,
                    "named_outer_handler_conditional_return.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_named_handler_teardown_before_intervening_finally() {
    for (scope, suspension) in [("def", ""), ("async def", "await ")] {
        for returned in ["inner", "None", "value"] {
            let source = format!(
                "{scope} example(value):\n    try:\n        {suspension}acquire()\n    except Exception as outer:\n        try:\n            {suspension}work()\n            return None\n        except Exception as inner:\n            return {returned}\n        finally:\n            {suspension}close()\n"
            );
            common::assert_matches_cpython_marshal(&source, "named_handler_intervening_finally.py");
        }
        let source = format!(
            "{scope} control():\n    try:\n        {suspension}acquire()\n    except Exception:\n        try:\n            {suspension}work()\n            return None\n        except Exception as inner:\n            return inner\n        finally:\n            {suspension}close()\n"
        );
        common::assert_matches_cpython_marshal(&source, "unnamed_outer_intervening_finally.py");
    }
}

#[test]
fn matches_cpython_two_fallback_handlers_inside_ordinary_try() {
    for value in ["0", "{}", "finish()"] {
        let source = format!(
            "def f():\n    try:\n        try:\n            return first()\n        except Exception:\n            try:\n                return second()\n            except Exception:\n                return {value}\n    except Exception:\n        return None\n"
        );
        common::assert_matches_cpython_marshal(&source, "two_fallback_handlers_outer_try.py");
    }
}
