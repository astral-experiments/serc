use crate::common;

#[test]
fn matches_cpython_marshal_for_copied_finally_local_order() {
    let source = r"def conditional_return(flag):
    try:
        if flag:
            return flag
        later = 1
    finally:
        cleanup = 2

async def return_before_handler(work):
    try:
        result = await work()
        return result
    except Exception as error:
        report(error)
        raise
    finally:
        elapsed = timer()
        report(elapsed)
";
    common::assert_matches_cpython_marshal(source, "copied_finally_local_order.py");
}

#[test]
fn matches_cpython_generator_finally_local_after_raising_assignment() {
    let source = r"def f():
    try:
        prepare()
        value = create()
        yield value
    except Exception:
        report()
        raise
    finally:
        before_cleanup()
        cleanup(value)
";
    common::assert_matches_cpython_marshal(
        source,
        "generator_finally_local_after_raising_assignment.py",
    );
}

#[test]
fn matches_cpython_async_generator_finally_local_after_raising_assignment() {
    let source = r"async def f(stream):
    try:
        start = timer()
        async for item in stream:
            yield item
    except TimeoutError as error:
        report()
        raise error
    finally:
        elapsed = timer() - start
        cleanup(elapsed)
";
    common::assert_matches_cpython_marshal(
        source,
        "async_generator_finally_local_after_raising_assignment.py",
    );
}

#[test]
fn matches_cpython_coroutine_finally_locals_after_caught_exception() {
    let source = r"async def f(manager):
    try:
        client = create()
        context = client.context()
        await work()
    except Exception:
        pass
    finally:
        with manager:
            cleanup(client, context)
";
    common::assert_matches_cpython_marshal(
        source,
        "coroutine_finally_locals_after_caught_exception.py",
    );
}

#[test]
fn matches_cpython_generator_finally_preinitialized_local() {
    let source = r"def f(value):
    try:
        prepare()
        value = create()
        yield value
    except Exception:
        report()
        raise
    finally:
        before_cleanup()
        cleanup(value)
";
    common::assert_matches_cpython_marshal(source, "generator_finally_preinitialized_local.py");
}

#[test]
fn matches_cpython_generator_finally_local_without_except() {
    let source = r"def f():
    try:
        prepare()
        value = create()
        yield value
    finally:
        before_cleanup()
        cleanup(value)
";
    common::assert_matches_cpython_marshal(source, "generator_finally_local_without_except.py");
}

#[test]
fn matches_cpython_ordinary_finally_local_after_raising_assignment() {
    let source = r"def f():
    try:
        prepare()
        value = create()
        work(value)
    except Exception:
        report()
        raise
    finally:
        before_cleanup()
        cleanup(value)
";
    common::assert_matches_cpython_marshal(
        source,
        "ordinary_finally_local_after_raising_assignment.py",
    );
}
