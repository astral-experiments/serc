use crate::common;

#[test]
fn matches_cpython_small_return_copies_after_while_breaks() {
    for prefix in ["", "async "] {
        for condition in ["flag", "flag and value"] {
            for body in [
                "if value:\n    break",
                "work()\nif value:\n    break",
                "work(); break",
            ] {
                for returned in [
                    "return",
                    "return value",
                    "return value + 1",
                    "return value.method()",
                    "return ready(value)",
                ] {
                    let source = format!(
                        "{prefix}def f(flag, value):\n    while {condition}:\n        {}\n    {returned}\n",
                        body.replace('\n', "\n        "),
                    );
                    common::assert_matches_cpython_marshal(&source, "while_break_return_copies.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_shared_implicit_while_break_returns() {
    for (prefix, suspension) in [
        ("", ""),
        ("async ", ""),
        ("", "    yield value\n"),
        ("async ", "    yield value\n"),
    ] {
        for condition in [
            "flag and value",
            "flag and value and other",
            "(flag or value) and other",
            "flag and (value or other)",
            "not (flag or value)",
            "flag and 0 < value < 4",
            "(saved := flag) and value",
            "flag or value",
        ] {
            for body in ["if other:\n    break", "if other:\n    break\nwork()"] {
                let source = format!(
                    "{prefix}def f(flag, value, other):\n{suspension}    while {condition}:\n        {}\n",
                    body.replace('\n', "\n        "),
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "shared_implicit_while_break_returns.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_breaks_before_finalizers_and_outer_backedges() {
    for (prefix, suspension) in [
        ("", ""),
        ("async ", ""),
        ("", "    yield value\n"),
        ("async ", "    yield value\n"),
    ] {
        let awaited = if prefix.is_empty() { "" } else { "await " };
        for cleanup in [format!("{awaited}cleanup()"), "pass".to_owned()] {
            let source = format!(
                "{prefix}def f(value, flag):\n{suspension}    try:\n        while True:\n            {awaited}work()\n            if flag:\n                break\n    finally:\n        {cleanup}\n",
            );
            common::assert_matches_cpython_marshal(&source, "break_before_finalizer.py");
        }

        for loop_else in ["", "        else:\n            return\n"] {
            let source = format!(
                "{prefix}def f(value, values):\n{suspension}    for item in values:\n        work(item)\n        while True:\n            break\n{loop_else}",
            );
            common::assert_matches_cpython_marshal(&source, "break_before_outer_backedge.py");
        }
    }
}

#[test]
fn matches_cpython_terminal_while_breaks_after_work() {
    for (prefix, suspension) in [
        ("", ""),
        ("async ", "        await pause()\n"),
        ("", "        yield\n"),
        ("async ", "        yield\n"),
    ] {
        for loop_condition in ["True", "running"] {
            for condition in ["value != limit", "value is None", "flag or running"] {
                for body in ["report(value)", "result = value"] {
                    let source = format!(
                        "{prefix}def f(value, limit, flag, running):\n    while {loop_condition}:\n{suspension}        tick()\n        if flag:\n            break\n        if {condition}:\n            {body}\n            break\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "terminal_while_break_after_work.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_terminal_while_breaks_after_compound_work() {
    for condition in ["value", "value != limit"] {
        for body in [
            "if early:\n    report()",
            "if early:\n    continue\nreport()",
            "with manager:\n    report()",
            "try:\n    report()\nfinally:\n    cleanup()",
        ] {
            let source = format!(
                "def f(value, limit, early, manager):\n    while True:\n        tick()\n        if {condition}:\n            {}\n            break\n",
                body.replace('\n', "\n            "),
            );
            common::assert_matches_cpython_marshal(
                &source,
                "terminal_while_break_after_compound_work.py",
            );
        }
    }
}

#[test]
fn matches_cpython_terminal_while_chained_breaks_after_work() {
    for (prefix, suspension) in [
        ("", ""),
        ("async ", "        await pause()\n"),
        ("", "        yield\n"),
        ("async ", "        yield\n"),
    ] {
        for loop_condition in ["True", "running"] {
            for condition in [
                "left <= value < limit",
                "not (left <= value < limit)",
                "left is value is None",
            ] {
                for body in ["report(value)", "result = value"] {
                    let source = format!(
                        "{prefix}def f(left, value, limit, stop, running):\n    while {loop_condition}:\n{suspension}        tick()\n        if stop:\n            break\n        if {condition}:\n            {body}\n            break\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "terminal_while_chained_break_after_work.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_terminal_while_else_chained_break_returns() {
    let source = "def f(items):\n    while items:\n        if a < value < b:\n            break\n        finish()\n    else:\n        tail()\n";
    for condition in [
        "a < value < b",
        "not a < value < b",
        "flag and a < value < b",
        "flag or a < value < b",
        "a < value is not None",
        "a < value < b and flag",
        "a < value < b or flag",
        "a < value",
    ] {
        common::assert_matches_cpython_marshal(
            &source.replace("a < value < b", condition),
            "terminal_while_else_chained_break_condition.py",
        );
    }
    for condition in ["items", "True", "False", "active()", "items and active"] {
        common::assert_matches_cpython_marshal(
            &source.replace("while items:", &format!("while {condition}:")),
            "terminal_while_else_chained_break_while_condition.py",
        );
    }
    for (filename, source) in [
        (
            "terminal_async_while_else_chained_break.py",
            source.replacen("def f", "async def f", 1),
        ),
        (
            "terminal_generator_while_else_chained_break.py",
            source.replace("    while items:", "    yield 1\n    while items:"),
        ),
        (
            "terminal_while_else_chained_break_to_name.py",
            format!("{source}    return name\n"),
        ),
        (
            "terminal_while_else_chained_break_to_local.py",
            format!("{source}    return items\n"),
        ),
        (
            "terminal_while_chained_break_to_name.py",
            format!(
                "{}    return name\n",
                source.replace("    else:\n        tail()\n", "")
            ),
        ),
        (
            "terminal_while_else_chained_break_nested_if.py",
            "def f(items):\n    if outer:\n        while items:\n            if a < value < b:\n                break\n            finish()\n        else:\n            tail()\n".to_owned(),
        ),
        (
            "nonterminal_while_else_chained_break.py",
            format!("{source}    after()\n"),
        ),
        (
            "terminal_while_else_chained_break_to_authored_return.py",
            format!("{source}    return\n"),
        ),
        (
            "terminal_while_else_chained_break_with_work.py",
            source.replace("            break", "            work()\n            break"),
        ),
    ] {
        common::assert_matches_cpython_marshal(&source, filename);
    }
}

#[test]
fn original_loop_exit_bridges_preserve_local_and_cell_return_copies() {
    for (filename, source) in [
        (
            "original_loop_bridge_coroutine_local_return_target.py",
            r"async def collect(mode, pending, fetch, stop):
    result = []
    if mode:
        while pending():
            value = await fetch()
            result.append(value)
            if stop(value):
                break
    else:
        result.append(await fetch())
    return result
",
        ),
        (
            "original_loop_bridge_cell_return_target.py",
            r"def collect(mode, take):
    count = 0
    def below_limit():
        return count < 4
    if mode:
        while below_limit():
            value = take()
            count += 1
            if value is None:
                break
    else:
        count = 1
    return count
",
        ),
        (
            "original_loop_bridge_local_return_target.py",
            r"def collect(mode, pending, take, stop):
    result = []
    if mode:
        while pending():
            value = take()
            result.append(value)
            if stop(value):
                break
    else:
        result.append(None)
    return result
",
        ),
        (
            "original_loop_bridge_cleanup_region_control.py",
            r"def collect(mode, pending, take, stop, release):
    result = []
    if mode:
        try:
            while pending():
                value = take()
                result.append(value)
                if stop(value):
                    break
        finally:
            release()
    else:
        result.append(None)
    return result
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_terminal_while_else_shared_return_footer() {
    for (filename, source) in [
        (
            "terminal_while_else_break_shared_footer.py",
            r"def run(short, items, work):
    if short:
        work(items)
    else:
        j = 0
        while j < len(items):
            a, b = items[j]
            if a > 0 and b > 0:
                work(a)
                work(b)
                break
            j += 1
        else:
            if a > 0:
                work(a)
            if b > 0:
                work(b)
",
        ),
        (
            "nonterminal_while_else_break_shared_footer_control.py",
            r"def run(short, items, work):
    if short:
        work(items)
    else:
        j = 0
        while j < len(items):
            a, b = items[j]
            if a > 0 and b > 0:
                work(a)
                work(b)
                break
            j += 1
        else:
            if a > 0:
                work(a)
            if b > 0:
                work(b)
    work(items)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_terminal_while_else_conjunction_return_copies() {
    common::assert_matches_cpython_marshal(
        r"def run(short, items, work):
    if short:
        work(items)
    else:
        j = 0
        while j < len(items):
            a, b = items[j]
            if a > 0 and b > 0:
                work(a)
                work(b)
                break
            j += 1
        else:
            if a > 0:
                work(a)
            if (
                a > 0
                and b > 0
                and not work(a)
                and work(b)
                and not work(a, b)
            ):
                work(a, b)
",
        "terminal_while_else_conjunction_return_copies.py",
    );
}

#[test]
fn constant_loop_bridges_preserve_small_return_line_events() {
    for prefix in ["", "async "] {
        for returned in ["x", "(x, flag)", "(x, flag, x)"] {
            let source = format!(
                "{prefix}def f(x, flag):\n    if flag:\n        while True:\n            x = g(x)\n            if x:\n                break\n    else:\n        x = h(x)\n    return {returned}\n",
            );
            common::assert_matches_cpython_marshal(&source, "constant_loop_bridge_return.py");
        }
    }
}
