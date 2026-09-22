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
fn named_handler_original_cleanup_uses_surviving_store_location() {
    for (filename, source) in [
        (
            "named_handler_reversed_store_pair.py",
            r"async def sample(flag, work, report):
    try:
        await work()
    except ValueError as error:
        if flag:
            report(error)
            left, right = None, None
        else:
            report(error)
            left, right = None, None
    except Exception:
        left, right = None, None
    return left, right
",
        ),
        (
            "named_handler_forward_store_pair_control.py",
            r"async def sample(flag, work, report):
    try:
        await work()
    except ValueError as error:
        if flag:
            report(error)
            left = right = None
        else:
            report(error)
            left = right = None
    except Exception:
        left = right = None
    return left, right
",
        ),
        (
            "named_handler_multiline_store_pair_control.py",
            r"async def sample(flag, work, report):
    try:
        await work()
    except ValueError as error:
        if flag:
            report(error)
            (left,
             right) = None, None
        else:
            report(error)
            (left,
             right) = None, None
    except Exception:
        left, right = None, None
    return left, right
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn named_handler_loop_exits_copy_original_name_cleanup() {
    for (filename, source) in [
        (
            "named_handler_nested_retry_break_target.py",
            r"def run(items, retries, work, report):
    for item in items:
        try:
            work(item)
        except LookupError as caught:
            report(item)
            for retry in retries:
                try:
                    work(item)
                    break
                except LookupError as caught:
                    report(retry)
                except Exception as caught:
                    report(item)
                    break
            else:
                report(item)
        report(item)
",
        ),
        (
            "async_named_handler_multi_break_target.py",
            r"async def run(items, work, report):
    try:
        await work(None)
    except LookupError as caught:
        report(None)
        for item in items:
            value = await work(item)
            if value is None:
                report(item)
                break
            if value:
                break
            try:
                await work(value)
                break
            except ValueError:
                report(value)
        else:
            await work(None)
    report(None)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn nested_named_handler_arms_share_the_original_cleanup() {
    for (filename, source) in [
        (
            "named_nested_assignment_cleanup_target.py",
            r"async def run(operation, ready, choose, record):
    result = None
    try:
        result = operation()
    except ValueError as error:
        record(error)
        token = operation()
        if token and ready(token):
            fallback = operation()
            if choose(fallback):
                result = error.value
            else:
                result = fallback
    await record(result)
    return result
",
        ),
        (
            "named_linear_assignment_cleanup_control.py",
            r"async def run(operation, ready, choose, record):
    result = None
    try:
        result = operation()
    except ValueError as error:
        record(error)
        token = operation()
        if token and ready(token):
            fallback = operation()
            result = fallback
    await record(result)
    return result
",
        ),
        (
            "named_nested_call_cleanup_target.py",
            r"async def run(operation, manager, ready, choose, enabled, record):
    try:
        with manager:
            result = await operation()
    except ValueError as error:
        result = error
        if ready():
            if choose(error):
                if enabled():
                    record(error)
            else:
                record(error, result)
    await record(result)
    return result
",
        ),
        (
            "named_linear_call_cleanup_control.py",
            r"async def run(operation, manager, ready, choose, enabled, record):
    try:
        with manager:
            result = await operation()
    except ValueError as error:
        result = error
        if ready():
            record(error, result)
    await record(result)
    return result
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn named_except_star_conditional_exits_copy_name_cleanup() {
    for (filename, source) in [
        (
            "named_except_star_conditional_cleanup_copy_target.py",
            r"async def run(work, choose, report):
    try:
        await work()
    except* ValueError as group:
        if choose(group):
            report(group)
        else:
            assert len(group.exceptions) == 1
    except* TypeError:
        raise
",
        ),
        (
            "named_except_star_linear_cleanup_copy_control.py",
            r"async def run(work, choose, report):
    try:
        await work()
    except* ValueError as group:
        report(group)
        assert len(group.exceptions) == 1
    except* TypeError:
        raise
",
        ),
        (
            "named_except_star_elif_cleanup_copy_target.py",
            r"async def run(work, choose, report):
    try:
        await work()
    except* ValueError as group:
        if choose(group, 0):
            report(group, 0)
        elif choose(group, 1):
            report(group, 1)
        else:
            assert len(group.exceptions) == 1
            report(group, 2)
    except* TypeError:
        raise
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
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

#[test]
fn protected_copy_plan_preserves_inner_branch_and_handler_cleanup_order() {
    common::assert_matches_cpython_marshal(
        "def read(a, b, c, inner, enabled):
    left = []
    right = []
    try:
        if a:
            left.append(a)
        elif b:
            if inner:
                left.append(b)
            else:
                right.append(b)
        elif c:
            left.append(c)
    except Exception as error:
        if enabled:
            left.append(a)
        else:
            right.append(b)
    return left, right
",
        "nested_handler_cleanup_copy_order.py",
    );
}
