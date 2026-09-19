use std::fmt::Write;

use crate::common;

#[test]
fn matches_cpython_terminal_while_constant_registration_order() {
    for definition in ["def", "async def"] {
        for prefix in ["'doc'", "work('before')"] {
            for condition in ["True", "1"] {
                for conditional_break in [false, true] {
                    let (leading, trailing) = if conditional_break {
                        ("        if stop:\n            break\n", "")
                    } else {
                        ("", "        break\n")
                    };
                    let source = format!(
                        "{definition} f():\n    {prefix}\n    while {condition}:\n\
                         {leading}        work(None)\n        work({condition})\n{trailing}"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "terminal_while_constant_order.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_first_keyword_constant_at_method_call_boundaries() {
    for definition in ["def", "async def"] {
        let await_prefix = if definition == "async def" {
            "await "
        } else {
            ""
        };
        for shape in ["discarded", "direct", "guarded"] {
            for count in [1, 15, 16, 28, 29, 30, 31] {
                for (callable, prefix) in [
                    ("obj.call", ""),
                    ("call", ""),
                    ("module.call", "import module\n"),
                ] {
                    let mut arguments = String::new();
                    for index in 0..count {
                        if index > 0 {
                            arguments.push_str(", ");
                        }
                        write!(arguments, "key{index}=value").unwrap();
                    }
                    let call = format!("{await_prefix}{callable}({arguments})");
                    let body = match shape {
                        "discarded" => format!("    raise Error\n    {call}\n"),
                        "direct" => format!(
                            "    while flag():\n        {call}\n        if flag():\n            break\n        other()\n"
                        ),
                        _ => format!(
                            "    while flag():\n        try:\n            {call}\n        except Error:\n            pass\n        if flag():\n            break\n        other()\n"
                        ),
                    };
                    let source = format!("{prefix}{definition} f(flag, value, obj):\n{body}");
                    common::assert_matches_cpython_marshal(
                        &source,
                        "first_keyword_method_call_boundaries.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_first_keyword_constant_before_terminal_await() {
    for definition in ["def", "async def"] {
        let await_prefix = if definition == "async def" {
            "await "
        } else {
            ""
        };
        for prefix in ["", "    'documentation'\n"] {
            for call in [
                "call(key=value)",
                "obj.call(key=value)",
                "call(*items, key=value)",
                "call(key=value, **options)",
                "call('literal', key=value)",
            ] {
                for discarded in [false, true] {
                    let body = if discarded {
                        format!("    raise Error\n    {await_prefix}{call}\n")
                    } else {
                        format!(
                            "    while flag():\n        try:\n            {await_prefix}{call}\n        except Error:\n            pass\n        if flag():\n            break\n        {await_prefix}other()\n"
                        )
                    };
                    let source = format!(
                        "{definition} f(flag, value, items, options, obj):\n{prefix}{body}"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "first_keyword_constant_before_await.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_terminal_while_await_constant_registration_order() {
    for prefix in ["", "    'doc'\n", "    consume('before')\n"] {
        for condition in ["running()", "not stopped()"] {
            for awaited in ["work()", "work('argument')", "work(0)"] {
                for before_await in ["", "        consume('earlier')\n"] {
                    let source = format!(
                        "async def f():\n{prefix}    while {condition}:\n{before_await}        await {awaited}\n        if stop():\n            break\n        consume('later')\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "terminal_while_await_constant_order.py",
                    );
                }
            }
        }
    }
}
