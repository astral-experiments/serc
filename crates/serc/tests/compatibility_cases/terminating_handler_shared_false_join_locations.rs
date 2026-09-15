use crate::common;

#[test]
fn matches_cpython_terminating_handler_shared_false_join_locations() {
    for (function, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield a\n"),
        ("async def", "    yield a\n"),
    ] {
        // Repeated operands still emit separate edges into the original cleanup block.
        for condition in [
            "a and b",
            "a or (b and c)",
            "not (a or b)",
            "a and a",
            "a or (b and b)",
        ] {
            for action in ["raise", "return"] {
                let source = format!(
                    "{function} f(a, b, c):\n{suspension}    try:\n        work()\n    except Exception as error:\n        if {condition}:\n            {action}\n    after()\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "terminating_handler_shared_false_join_locations.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_terminating_handler_single_false_edge_locations() {
    for (function, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield a\n"),
        ("async def", "    yield a\n"),
    ] {
        for condition in ["a or b", "a"] {
            let source = format!(
                "{function} f(a, b):\n{suspension}    try:\n        work()\n    except Exception as error:\n        if {condition}:\n            raise\n    after()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "terminating_handler_single_false_edge_locations.py",
            );
        }
    }
}

#[test]
fn matches_cpython_terminating_handler_shared_false_join_before_else() {
    for condition in ["a or (b and c)", "not (a or b)"] {
        for action in ["raise", "return"] {
            let source = format!(
                "async def f(a, b, c):\n    try:\n        work()\n    except Exception as error:\n        if {condition}:\n            {action}\n    else:\n        observe()\n    after()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "terminating_handler_shared_false_join_before_else.py",
            );
        }
    }
}

#[test]
fn matches_cpython_terminal_unnamed_handler_shared_cleanup() {
    for (filename, source) in [
        (
            "unnamed_async_and.py",
            r"async def run_handler(action, current_task):
    try:
        await action()
    except CancelledError:
        task = current_task()
        if task is not None and task.cancelling():
            raise
",
        ),
        (
            "unnamed_async_or_and.py",
            r"async def run_handler(action, current_task, terminal_seen):
    try:
        await action()
    except CancelledError:
        task = current_task()
        if not terminal_seen or (task is not None and task.cancelling()):
            raise
",
        ),
        (
            "unnamed_single_false_control.py",
            r"async def run_handler(action, current_task):
    try:
        await action()
    except CancelledError:
        task = current_task()
        if task is not None:
            raise
",
        ),
        (
            "unnamed_pass_control.py",
            r"async def run_handler(action, current_task):
    try:
        await action()
    except CancelledError:
        task = current_task()
        if task is not None and task.cancelling():
            raise
        pass
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_handler_final_else_condition_location() {
    let source = r"def retry(attempts, send, log, special):
    for attempt in range(attempts):
        try:
            response = send()
            log(response)
        except LookupError as error:
            if special(error):
                log(error)
                return None
            else:
                log(error)
                if attempt == attempts - 1:
                    return None
        except Exception as error:
            log(error)
            if attempt == attempts - 1:
                return None
        else:
            assert isinstance(response, dict)
            return response
    raise AssertionError
";
    common::assert_matches_cpython_marshal(source, "handler_final_else_condition_location.py");
}

#[test]
fn matches_cpython_handler_final_else_multiple_predecessors() {
    let source = r"def retry(attempts, send, log, special):
    for attempt in range(attempts):
        try:
            response = send()
            log(response)
        except LookupError as error:
            if special(error):
                log(error)
            else:
                log(error)
                if attempt == attempts - 1:
                    return None
        except Exception as error:
            log(error)
            if attempt == attempts - 1:
                return None
        else:
            assert isinstance(response, dict)
            return response
    raise AssertionError
";
    common::assert_matches_cpython_marshal(source, "handler_final_else_multiple_predecessors.py");
}
