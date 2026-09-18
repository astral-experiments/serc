use crate::common;

#[test]
fn matches_cpython_returns_after_copied_finally_managers() {
    for (definition, prologue) in [("def", ""), ("def", "    yield value\n"), ("async def", "")] {
        for returned in ["value", "work(value)"] {
            for prefix in ["", "        cleanup()\n"] {
                for body in ["pass", "consume(value)"] {
                    let source = format!(
                        "{definition} collect(value, manager):\n{prologue}    try:\n        return {returned}\n    finally:\n{prefix}        with manager:\n            {body}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "copied_finally_manager_return.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_shared_iterator_return_value_cleanup_prefix_locations() {
    for source in [
        r"def f(items, outer, inner, flag):
    with outer:
        for item in items:
            try:
                return []
            finally:
                if flag:
                    with inner:
                        cleanup()
",
        r"def f(items, outer, inner, flag):
    with outer:
        for item in items:
            try:
                return []
            finally:
                if flag:
                    cleanup()
",
        r"def f(items, outer, inner, flag):
    for item in items:
        try:
            return []
        finally:
            if flag:
                with inner:
                    cleanup()
",
    ] {
        common::assert_matches_cpython_marshal(source, "shared_iterator_cleanup_prefix.py");
    }
}

#[test]
fn constant_manager_return_cleanup_owners_match_cpython() {
    for (filename, source) in [
        (
            "constant_iterator_manager_conditional_return.py",
            r"async def example(manager, items, value):
    try:
        with manager:
            for item in items:
                try:
                    if should_return(item):
                        tick()
                        return
                except ValueError:
                    pass
    except RuntimeError:
        raise
    finally:
        if value is not None:
            finish(value)
        after()
",
        ),
        (
            "constant_named_handler_manager_await_yield_keyword.py",
            r"async def example(manager, value):
    try:
        with manager:
            try:
                work()
            except ValueError as error:
                await cancel()
                yield Event(text='failure')
                return
    finally:
        if value is not None and not value.done():
            finish(value)
        after()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn copied_finally_retained_manager_return_owners_match_cpython() {
    for (filename, source) in [
        (
            "copied_finally_inner_and_outer_manager.py",
            r"def exercise(outer, inner, value):
    with outer:
        try:
            with inner():
                if value is not None:
                    work(value)
                    return
                work(None)
            after()
        except Exception as error:
            if value is not None:
                record(error)
            raise
        finally:
            if value is not None:
                finish(value)
",
        ),
        (
            "copied_finally_inner_manager_only.py",
            r"def exercise(outer, inner, value):
    try:
        with inner():
            if value is not None:
                work(value)
                return
            work(None)
        after()
    except Exception as error:
        if value is not None:
            record(error)
        raise
    finally:
        if value is not None:
            finish(value)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
