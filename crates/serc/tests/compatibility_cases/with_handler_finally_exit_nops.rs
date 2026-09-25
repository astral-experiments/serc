use crate::common;

#[test]
fn matches_cpython_with_exit_after_handler_finally() {
    for (prefix, suspension) in [
        ("", ""),
        ("", "    yield value\n"),
        ("async ", "    await before()\n"),
    ] {
        for body in ["work(value)", "with inner():\n    work(value)"] {
            for handler in ["report()", "raise", "report()\nreturn", "pass"] {
                for cleanup in ["cleanup()", "value = None", "cleanup()\nreport()"] {
                    let body = body.replace('\n', "\n            ");
                    let handler = handler.replace('\n', "\n            ");
                    let cleanup = cleanup.replace('\n', "\n            ");
                    let source = format!(
                        "{prefix}def example(manager, value):\n{suspension}    with manager:\n        try:\n            {body}\n        except Error:\n            {handler}\n        finally:\n            {cleanup}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "with_handler_finally_exit_nops.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_with_finally_branch_join_ownership() {
    for (prefix, suspension, body) in [
        ("", "", "work(value)"),
        ("", "", "for item in values:\n    work(item)"),
        (
            "",
            "    yield value\n",
            "for item in values:\n    work(item)",
        ),
        (
            "async ",
            "    await before()\n",
            "for item in values:\n    work(item)",
        ),
    ] {
        for after in ["", "    after()\n"] {
            let body = body.replace('\n', "\n            ");
            let source = format!(
                "{prefix}def example(manager, value, values, flag):\n{suspension}    with manager:\n        try:\n            {body}\n        except Error:\n            raise\n        finally:\n            if flag:\n                cleanup()\n            else:\n                report()\n{after}"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "with_finally_branch_join_ownership.py",
            );
        }
    }
}

#[test]
fn matches_cpython_manager_pop_after_returning_try_handler_finally() {
    for handler in ["await report(error)", "raise", "return"] {
        let source = format!(
            "async def example(manager, flag):\n    with manager:\n        await prepare()\n        try:\n            result = await work()\n            return result\n        except Exception as error:\n            {handler}\n        finally:\n            if flag:\n                mark()\n            finish()\n"
        );
        common::assert_matches_cpython_marshal(&source, "manager_pop_returning_try.py");
    }
}

#[test]
fn matches_cpython_manager_pop_after_generator_try_else_finally() {
    for (handler, otherwise) in [
        ("raise", "await succeeded()"),
        ("raise", "return"),
        ("raise", "raise Error"),
        ("await failed()", "return"),
    ] {
        let source = format!(
            "async def example(manager, obj):\n    with manager:\n        old = obj.value\n        try:\n            await start()\n            yield obj\n        except Exception:\n            {handler}\n        else:\n            {otherwise}\n        finally:\n            obj.value = old\n"
        );
        common::assert_matches_cpython_marshal(&source, "manager_pop_generator_else.py");
    }
}

#[test]
fn folded_true_finally_call_drops_redundant_manager_exit() {
    for condition in ["__debug__", "True", "1"] {
        let source = format!(
            "def probe(outer, inner, work, log):\n    with outer:\n        try:\n            with inner:\n                work()\n        finally:\n            if {condition}:\n                log('finished')\n"
        );
        common::assert_matches_cpython_marshal(&source, "folded_true_finally_call.py");
    }
}

#[test]
fn matches_cpython_nested_pass_finally_continuation() {
    for continuation in ["after()", "if other:\n    after()"] {
        let continuation = continuation.replace('\n', "\n        ");
        let source = format!(
            "def example(flag, other):\n    try:\n        yield 1\n    finally:\n        try:\n            if flag:\n                first()\n            else:\n                second()\n        finally:\n            pass\n        {continuation}\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_pass_finally_continuation.py");
    }
}

#[test]
fn matches_cpython_manager_pop_after_conditional_for_exhaustion() {
    let source = "def example(manager, values, flag):\n    with manager:\n        if flag:\n            for value in values:\n                consume(value)\n        else:\n            raise Error\n    finish()\n";
    common::assert_matches_cpython_marshal(source, "manager_pop_conditional_for.py");
}

#[test]
fn matches_cpython_nested_manager_in_coroutine_finally() {
    for (prefix, suspension, inner) in [
        ("", "", "log()"),
        ("", "    yield value\n", "log()"),
        ("async ", "", "log()"),
        ("async ", "    await before()\n", "await log()"),
    ] {
        for condition in ["first and second", "first or second", "first"] {
            for after in ["", "    finish()\n"] {
                let source = format!(
                    "{prefix}def f():\n{suspension}    with context():\n        try:\n            action()\n        finally:\n            if {condition}:\n                with suppress(Exception):\n                    {inner}\n{after}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "nested_manager_coroutine_finally.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_nested_manager_in_async_generator_finally() {
    for inner in ["log()", "await log()"] {
        for after in ["", "    finish()\n"] {
            let source = format!(
                "async def f():\n    yield value\n    with context():\n        try:\n            action()\n        finally:\n            if first and second:\n                with suppress(Exception):\n                    {inner}\n{after}"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "nested_manager_async_generator_finally.py",
            );
        }
    }
}

#[test]
fn matches_cpython_suspended_chained_guarded_manager_finally() {
    for (scope, suspension) in [
        ("def", ""),
        ("def", "    yield value\n"),
        ("async def", ""),
        ("async def", "    await before()\n"),
    ] {
        for condition in ["first < second < third", "first < second"] {
            for work in ["log()", "work()\nlog()"] {
                for after in ["", "    finish()\n"] {
                    for outer in [true, false] {
                        let indent = if outer { "        " } else { "    " };
                        let manager = if outer { "    with context():\n" } else { "" };
                        let work = work.replace('\n', &format!("\n{indent}            "));
                        let source = format!(
                            "{scope} f():\n{suspension}{manager}{indent}try:\n{indent}    action()\n{indent}finally:\n{indent}    if {condition}:\n{indent}        with suppress(Exception):\n{indent}            {work}\n{after}"
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "generator_chained_guarded_manager.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_async_generator_guarded_manager_owner() {
    for condition in ["first or second", "first"] {
        for inner in ["log()", "await log()"] {
            for after in ["", "    finish()\n"] {
                let source = format!(
                    "async def f():\n    yield value\n    with context():\n        try:\n            action()\n        finally:\n            if {condition}:\n                with suppress(Exception):\n                    {inner}\n{after}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "async_generator_guarded_manager_owner.py",
                );
            }
        }
    }
}
