use crate::common;

#[test]
fn matches_cpython_marshal_for_manager_prefix_while_continuations() {
    for (filename, source) in [
        (
            "with_prefix_loop_before_finally.py",
            r"def f(manager, ready, work, cleanup):
    with manager:
        while not ready():
            work()
        try:
            work()
        finally:
            cleanup()
",
        ),
        (
            "async_function_with_prefix_loop_before_finally.py",
            r"async def f(manager, ready, work, cleanup):
    with manager:
        while not ready():
            await work()
        try:
            await work()
        finally:
            await cleanup()
",
        ),
        (
            "with_terminal_loop_outside_continuation_control.py",
            r"def f(manager, ready, work, after):
    with manager:
        while ready():
            work()
    after()
",
        ),
        (
            "with_prefix_for_before_finally_control.py",
            r"def f(manager, values, work, cleanup):
    with manager:
        for value in values:
            work(value)
        try:
            work(None)
        finally:
            cleanup()
",
        ),
        (
            "async_manager_prefix_loop_before_finally_control.py",
            r"async def f(manager, ready, work, cleanup):
    async with manager:
        while ready():
            await work()
        try:
            await work()
        finally:
            await cleanup()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_marshal_for_nested_manager_while_continuations() {
    for (filename, source) in [
        (
            "nested_if_while_try_with_continuation.py",
            r"def f(manager, ready, test, work, after):
    with manager:
        try:
            if ready():
                while test():
                    work()
            after()
        except LookupError:
            raise
",
        ),
        (
            "nested_if_while_async_with_finally_continuation.py",
            r"async def f(manager, ready, test, work, after, cleanup):
    async with manager:
        try:
            if ready():
                while test():
                    await work()
            after()
        finally:
            cleanup()
",
        ),
        (
            "nested_if_while_for_with_continuation.py",
            r"def f(manager, values, ready, test, work, after):
    with manager:
        for value in values:
            if ready(value):
                while test():
                    work()
            after(value)
",
        ),
        (
            "nested_else_while_for_with_continuation.py",
            r"def f(manager, values, ready, test, work, after):
    with manager:
        for value in values:
            if ready(value):
                work()
            else:
                while test():
                    work()
            after(value)
",
        ),
        (
            "double_nested_if_while_try_with_continuation.py",
            r"def f(manager, first, second, test, work, after):
    with manager:
        try:
            if first():
                if second():
                    while test():
                        work()
            after()
        except LookupError:
            raise
",
        ),
        (
            "nested_manager_terminal_while_inside_branch_control.py",
            r"def f(manager, nested, ready, test, work, after):
    with manager:
        try:
            if ready():
                with nested:
                    while test():
                        work()
            after()
        except LookupError:
            raise
",
        ),
        (
            "nested_try_terminal_while_inside_branch_control.py",
            r"def f(manager, ready, test, work, after):
    with manager:
        try:
            if ready():
                try:
                    while test():
                        work()
                except LookupError:
                    raise
            after()
        except TypeError:
            raise
",
        ),
        (
            "nested_if_while_terminal_body_control.py",
            r"def f(manager, ready, test, work):
    with manager:
        if ready():
            while test():
                work()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
