use std::fmt::Write;

use crate::common;

#[test]
fn terminal_try_async_manager_jumps_copy_exception_owners() {
    for prefix in [
        "",
        "    if flag:\n        return None\n",
        "    await prepare()\n",
    ] {
        for handler in [
            "except Error:\n        return None",
            "except Error as exc:\n        return failed(exc)",
            "except Error:\n        raise",
            "except Error:\n        pass",
        ] {
            let source = format!(
                "async def f(flag, manager, work):\n{prefix}    try:\n        if flag:\n            result = await work()\n        else:\n            async with manager() as session:\n                result = await work(session)\n    {handler}\n    if result.field is not None:\n        return result\n    return await finish(result)\n",
            );
            common::assert_matches_cpython_marshal(&source, "async_manager_try_exit.py");
        }
    }
}

#[test]
fn terminal_try_branch_jumps_inherit_the_manager_owner() {
    for prefix in ["", "async "] {
        let suspension = if prefix.is_empty() { "" } else { "await " };
        for branches in [2, 3] {
            for conditional in [false, true] {
                let value = if conditional {
                    format!("{suspension}work() if other_flag else {suspension}other()")
                } else {
                    format!("{suspension}work()")
                };
                let mut body = String::new();
                for branch in 0..branches {
                    let condition = if branch == 0 {
                        "if flag == 0:".to_owned()
                    } else if branch + 1 == branches {
                        "else:".to_owned()
                    } else {
                        format!("elif flag == {branch}:")
                    };
                    writeln!(
                        body,
                        "            {condition}\n                value = {value}"
                    )
                    .unwrap();
                }
                let source = format!(
                    "{prefix}def f(flag, other_flag):\n    with manager:\n        try:\n{body}        except Error:\n            {suspension}cleanup()\n            raise\n    return value\n",
                );
                common::assert_matches_cpython_marshal(&source, "terminal_try_branch_owner.py");
            }
        }
    }
}

#[test]
fn loop_control_keeps_manager_exits_in_the_enclosing_try() {
    for prefix in ["", "async "] {
        for loop_header in ["for value in values", "while flag"] {
            for action in ["break", "continue"] {
                for layout in ["loop_try_with", "try_loop_with", "with_loop_try"] {
                    let indent = |body: &str| format!("    {}", body.replace('\n', "\n    "));
                    let protect = |body: &str| {
                        format!("try:\n{}\nexcept Error:\n    recover()", indent(body))
                    };
                    let loop_body = |body: &str| format!("{loop_header}:\n{}", indent(body));
                    let manager = |body: &str| format!("with manager:\n{}", indent(body));
                    let body = format!("prepare()\nif flag:\n    {action}\nuse()");
                    let body = match layout {
                        "loop_try_with" => loop_body(&protect(&manager(&body))),
                        "try_loop_with" => protect(&loop_body(&manager(&body))),
                        "with_loop_try" => manager(&loop_body(&protect(&body))),
                        _ => unreachable!(),
                    };
                    let source =
                        format!("{prefix}def f(values, flag, manager):\n{}\n", indent(&body));
                    common::assert_matches_cpython_marshal(&source, "manager_loop_exit_owner.py");
                }
            }
        }
    }
}

#[test]
fn copied_finally_cleanup_jumps_keep_the_outer_manager_owner() {
    for (prefix, action) in [
        ("", "work()"),
        ("", "yield value"),
        ("async ", "await work()"),
        ("async ", "yield value"),
    ] {
        for cleanup in [
            "with cleanup_manager:\n    cleanup()",
            "try:\n    cleanup()\nexcept Error:\n    pass",
            "try:\n    cleanup()\nexcept Error as error:\n    pass",
            "try:\n    cleanup()\nexcept:\n    pass",
        ] {
            for following in ["", "        finish()\n"] {
                let source = format!(
                    "{prefix}def f():\n    with outer_manager:\n        try:\n            {action}\n        finally:\n            {}\n{following}",
                    cleanup.replace('\n', "\n            "),
                );
                common::assert_matches_cpython_marshal(&source, "copied_finally_cleanup_owner.py");
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_copied_finally_loop_control() {
    for action in ["break", "continue"] {
        let source = format!(
            "def f(items, flag):\n    for item in items:\n        try:\n            work(item)\n        except Error as error:\n            if flag:\n                {action}\n            report(error)\n        finally:\n            cleanup()\n"
        );
        common::assert_matches_cpython_marshal(&source, "copied_finally_loop_control.py");
    }
}

#[test]
fn matches_cpython_marshal_for_interleaved_finally_manager_loop_control() {
    let source = r"def f(items, flag, manager):
    for item in items:
        try:
            with manager:
                try:
                    work(item)
                    if flag:
                        break
                    continue
                finally:
                    cleanup_inner()
        finally:
            cleanup_outer()
";
    common::assert_matches_cpython_marshal(source, "interleaved_finally_manager_loop_control.py");
}

#[test]
fn matches_cpython_marshal_for_direct_handler_finally_loop_control() {
    for handler in ["Error", "Error as error"] {
        for action in ["break", "continue"] {
            let source = format!(
                "def f(items):\n    for item in items:\n        try:\n            work(item)\n        except {handler}:\n            {action}\n        finally:\n            cleanup()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "direct_handler_finally_loop_control.py",
            );
        }
    }
}

#[test]
fn matches_cpython_marshal_for_finally_inside_handler_loop_control() {
    for action in ["break", "continue"] {
        for handler in ["Error", "Error as error"] {
            let source = format!(
                "def f(items):\n    for item in items:\n        try:\n            work(item)\n        except {handler}:\n            try:\n                {action}\n            finally:\n                cleanup()\n    finished()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "finally_inside_handler_loop_control.py",
            );
        }
    }
}

#[test]
fn matches_cpython_marshal_for_finally_inside_handler_and_manager_loop_control() {
    for action in ["break", "continue"] {
        for handler in ["Error", "Error as error"] {
            let source = format!(
                "def f(items, manager):\n    for item in items:\n        try:\n            work(item)\n        except {handler}:\n            with manager:\n                try:\n                    {action}\n                finally:\n                    cleanup()\n    finished()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "finally_inside_handler_and_manager_loop_control.py",
            );
        }
    }
}

#[test]
fn executes_nested_finally_overrides_and_preserves_active_handler() {
    let source = r"import sys
from contextlib import nullcontext

trace = []
for item in range(2):
    try:
        try:
            break
        finally:
            if item == 0:
                continue
            trace.append('inner')
    finally:
        trace.append('outer')
print(trace)

for item in range(1):
    try:
        raise ValueError('active')
    except ValueError as error:
        try:
            break
        finally:
            print(sys.exception() is error)

def return_overridden_by_continue():
    for item in range(2):
        try:
            return item
        finally:
            if item == 0:
                continue
            print('return cleanup')

print(return_overridden_by_continue())

def return_through_manager():
    for item in range(2):
        try:
            return item
        finally:
            with nullcontext():
                if item == 0:
                    continue

def return_through_inner_loop():
    try:
        return 5
    finally:
        for item in range(1):
            break

print(return_through_manager(), return_through_inner_loop())
";
    common::assert_executes(
        source,
        "['outer', 'inner', 'outer']\nTrue\nreturn cleanup\n1\n1 5\n",
    );
}

#[test]
fn enclosing_finally_break_owners_match_cpython_marshal() {
    for scope in ["ordinary", "generator", "coroutine", "async_generator"] {
        for iterator in [false, true] {
            let prefix = if matches!(scope, "coroutine" | "async_generator") {
                "async "
            } else {
                ""
            };
            let suspension = if matches!(scope, "generator" | "async_generator") {
                "    yield None\n"
            } else {
                ""
            };
            let header = if iterator {
                "for value in values"
            } else {
                "while flag"
            };
            let source = format!(
                "{prefix}def f(values, flag):\n{suspension}    try:\n        {header}:\n            if check():\n                break\n            use()\n        use()\n    finally:\n        cleanup()\n"
            );
            common::assert_matches_cpython_marshal(&source, "enclosing_finally_break_owners.py");
        }
    }
}

#[test]
fn breaks_to_following_try_body_statements_keep_the_handler_owner() {
    for prefix in ["", "async "] {
        for header in ["for value in values", "while flag"] {
            for following in ["use()", "result = process(values)"] {
                let source = format!(
                    "{prefix}def f(values, flag):\n    try:\n        {header}:\n            if flag:\n                break\n            use()\n        {following}\n    except Error:\n        cleanup()\n"
                );
                common::assert_matches_cpython_marshal(&source, "break_following_try_body.py");
            }
        }
    }
}

#[test]
fn nested_break_targets_keep_only_their_enclosing_try_owner() {
    for prefix in ["", "async "] {
        for body in [
            "for outer in values:\n    for inner in values:\n        if flag:\n            break\n        use()",
            "if flag:\n    for inner in values:\n        if ready():\n            break\n        use()\nfinish()",
            "for item in values:\n    if flag:\n        break\n    use()",
        ] {
            let source = format!(
                "{prefix}def f(values, flag):\n    try:\n        {}\n    except Error:\n        recover()\n    finish()\n",
                body.replace('\n', "\n        "),
            );
            common::assert_matches_cpython_marshal(&source, "nested_break_target_owners.py");
        }
    }
}

#[test]
fn handler_finally_continues_keep_locations_inside_enclosing_tries() {
    for loop_header in ["for item in items", "while flag"] {
        for outer_cleanup in [
            "except Outer:\n    recover()",
            "finally:\n    outer_cleanup()",
        ] {
            let source = format!(
                "def f(items, flag):\n    try:\n        {loop_header}:\n            try:\n                work()\n            except Error as error:\n                try:\n                    continue\n                finally:\n                    cleanup()\n        finished()\n    {}\n",
                outer_cleanup.replace('\n', "\n    "),
            );
            common::assert_matches_cpython_marshal(
                &source,
                "outer_try_handler_finally_continue.py",
            );
        }
    }
}
