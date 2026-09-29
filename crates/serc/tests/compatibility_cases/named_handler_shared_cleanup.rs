use crate::common;

#[test]
fn named_handler_conditional_normal_exits_inline_name_cleanup() {
    for scope in ["def f():\n", "async def f():\n", "def f():\n    yield\n"] {
        for condition in ["ready()", "ready() and enabled()"] {
            for normal_arm in ["report()", "return\nelif other():\n    report()"] {
                for alternative in ["return", "raise", "fallback()"] {
                    for continuation in ["", "finish()\n"] {
                        let branch =
                            format!("if {condition}:\n    {normal_arm}\nelse:\n    {alternative}");
                        let suite = format!(
                            "try:\n    observe()\nexcept ValueError as error:\n    {}\n{continuation}",
                            branch.replace('\n', "\n    "),
                        );
                        let source =
                            format!("{scope}    {}", suite.trim_end().replace('\n', "\n    "));
                        common::assert_matches_cpython_marshal(
                            &source,
                            "named_handler_shared_cleanup.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn handler_while_cleanup_uses_normal_condition_exits() {
    for scope in [
        "def f(a, b):\n",
        "async def f(a, b):\n",
        "def f(a, b):\n    yield\n",
    ] {
        for name in ["", " as error"] {
            for condition in ["a", "a and b", "a or b"] {
                let source = format!(
                    "{scope}    try:\n        work()\n    except Exception{name}:\n        while {condition}:\n            value = step()\n    after()\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "handler_while_cleanup_condition.py",
                );
            }
        }
    }
}

#[test]
fn handler_cleanup_keeps_a_separately_lowered_loop_successor() {
    for name in ["", " as error"] {
        for condition in ["a", "True"] {
            let source = format!(
                "def f(a, b, c, items):\n    for item in items:\n        try:\n            work()\n        except Exception{name}:\n            if {condition}:\n                first = error\n        try:\n            work()\n        except Exception as error:\n            if b:\n                second = error\n    after()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "handler_cleanup_separate_loop_successor.py",
            );
        }
    }
}

#[test]
fn protected_copy_plans_include_predecessors_of_handler_cleanup_sources() {
    for handler in ["Exception as error", "Exception"] {
        for returned in ["left, right", "left, right, enabled"] {
            let source = format!(
                "def read(a, b, c, enabled):
    left = []
    right = []
    try:
        if a:
            left.append(a)
        elif b:
            right.append(b)
        elif c:
            left.append(c)
    except {handler}:
        if enabled:
            left.append(a)
        else:
            right.append(b)
    return {returned}
"
            );
            common::assert_matches_cpython_marshal(&source, "handler_cleanup_copy_order.py");
        }
    }
}
