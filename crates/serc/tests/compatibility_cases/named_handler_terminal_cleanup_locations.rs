use crate::common;

#[test]
fn matches_cpython_coroutine_named_complete_cleanup_joins() {
    for outer in ["flag", "flag and other"] {
        for prefix in ["", "            notify(error)\n"] {
            for (first, last) in [
                ("first(error)", "last(error)"),
                ("value = error", "value = None"),
            ] {
                let source = format!(
                    "async def f(flag, other):\n    try:\n        await work()\n    except Error as error:\n        if {outer}:\n{prefix}            if other:\n                {first}\n            else:\n                {last}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "coroutine_named_complete_cleanup.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_terminal_named_handler_original_false_joins() {
    for condition in ["flag and error.code", "flag or error.code", "flag"] {
        let source = format!(
            "def f(flag):\n    try:\n        work()\n        return\n    except ValueError as error:\n        if {condition}:\n            notify()\n            return\n    except Exception as error:\n        raise error\n"
        );
        common::assert_matches_cpython_marshal(&source, "terminal_named_handler_false_join.py");
    }
}

#[test]
fn matches_cpython_named_handler_runtime_break_return_copies() {
    for (function, poll) in [("def", "poll()"), ("async def", "await poll()")] {
        for returned in ["first", "first, second", "None"] {
            let source = format!(
                "{function} f(flag):\n    first = []\n    second = []\n    try:\n        work()\n    except Exception as error:\n        while flag:\n            events = {poll}\n            if not events:\n                break\n            first.append(events)\n    return {returned}\n"
            );
            common::assert_matches_cpython_marshal(&source, "named_handler_break_return_copy.py");
        }
    }
}
