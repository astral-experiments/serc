use crate::common;

#[test]
fn matches_cpython_shared_call_discard_after_optional_legend() {
    for function in ["def", "async def"] {
        for prefix in ["", "    yield legend\n"] {
            let source = format!(
                "{function} plot(legend, rotate, plt):\n{prefix}    if legend is not None:\n        plt.legend() if legend is True else plt.legend(legend)\n    if rotate is not None:\n        plt.rotate(rotate)\n    return\n"
            );
            common::assert_matches_cpython_marshal(&source, "optional_legend_call_discard.py");
        }
    }
}

#[test]
fn matches_cpython_shared_conditional_discard_before_outer_branch_exit() {
    for function in ["def", "async def"] {
        let source = format!(
            "{function} process(flag, obj):\n    if flag:\n        before()\n    else:\n        obj.item.remove() if obj.item else None\n    after()\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "shared_conditional_discard_outer_branch.py",
        );
    }
}

#[test]
fn matches_cpython_shared_conditional_discard_before_async_with_exit() {
    for prefix in ["", "    yield None\n"] {
        for nested in [false, true] {
            for fallback in ["None", "[]"] {
                let body = format!("work() if flag else {fallback}");
                let body = if nested {
                    format!("if other:\n    raise Error\nelse:\n    {body}")
                } else {
                    body
                };
                let source = format!(
                    "async def process(flag, other, manager):\n{prefix}    async with manager:\n        {}\n",
                    body.replace('\n', "\n        ")
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "shared_conditional_discard_async_with.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_conditional_discard_before_async_with_continuation() {
    for prefix in ["", "    yield None\n"] {
        for condition in ["flag", "True", "False"] {
            for tail in ["after()", "return"] {
                let source = format!(
                    "async def process(flag, manager):\n{prefix}    async with manager:\n        work() if {condition} else []\n        {tail}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "conditional_discard_async_with_continuation.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_conditional_expression_discard_before_return() {
    for (scope, prefix) in [("", ""), ("async ", ""), ("", "    yield flag\n")] {
        for managed in [false, true] {
            let conditions: &[&str] = if managed {
                &["flag", "True", "False"]
            } else {
                &["flag"]
            };
            for condition in conditions {
                for tail in ["return result", "return"] {
                    let body = format!("work() if {condition} else []\n{tail}");
                    let body = if managed {
                        format!("with manager:\n    {}", body.replace('\n', "\n    "))
                    } else {
                        body
                    };
                    let source = format!(
                        "{scope}def process(flag, result, manager):\n{prefix}    {}\n",
                        body.replace('\n', "\n    ")
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "conditional_expression_discard.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_loop_conditional_expression_exits() {
    for leading in ["", "left and ", "left or ", "not "] {
        for body in ["right", "right > value"] {
            for fallback in ["0", "1", "other"] {
                for multiline in [false, true] {
                    let conditional = if multiline {
                        format!(
                            "\n            {body}\n            if selector\n            else {fallback}\n        "
                        )
                    } else {
                        format!("{body} if selector else {fallback}")
                    };
                    let source = format!(
                        "def f(items, left, right, selector, other, value):\n    for item in items:\n        if {leading}({conditional}):\n            work(item)\n    return value\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "loop_conditional_expression_exits.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_shared_conditional_discard_at_loop_branch_tail() {
    for branches in [
        "",
        "        if line == 0:\n            consume(line)\n        else:\n",
        "        if line == 0:\n            consume(line)\n        elif line == 1:\n            continue\n        elif line == 2:\n            results.append(line)\n        else:\n",
    ] {
        let indent = if branches.is_empty() {
            "        "
        } else {
            "            "
        };
        let source = format!(
            "def parse(lines, results):\n    for line in lines:\n{branches}{indent}row = convert(line)\n{indent}results.append(row) if row else None\n    return results\n"
        );
        common::assert_matches_cpython_marshal(&source, "loop_branch_conditional_discard.py");
    }
}

#[test]
fn matches_cpython_borrow_runtime_conditional_loop_branch_entry() {
    for condition in [
        "value < other if flag else value > other",
        "check(value) if check(flag) else check(other)",
    ] {
        let source = format!(
            "def f(flag, value, other, owner, items, check):\n    for item in items:\n        if {condition}:\n            owner.append(value)\n        else:\n            owner.append(other)\n    return owner\n"
        );
        common::assert_matches_cpython_marshal(&source, "runtime_if_expression_loop_entry.py");
    }
}
