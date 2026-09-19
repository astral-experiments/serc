use crate::common;

#[test]
fn matches_cpython_marshal_for_terminal_for_finally_target() {
    let source = r"def make_worker(manager, finish):
    async def worker(items, fail_fast):
        try:
            async with manager:
                raw = await fetch()
            for item in items:
                consume(item, raw)
        except Exception as error:
            if fail_fast:
                raise
            for item in items:
                recover(item, error)
        finally:
            finish(len(items))

    return worker
";
    common::assert_matches_cpython_marshal(source, "terminal_for_finally_target.py");
}

#[test]
fn matches_cpython_marshal_for_terminal_for_finally_handler_control() {
    let source = r"def make_worker(manager, finish):
    async def worker(items, fail_fast):
        try:
            async with manager:
                raw = await fetch()
            for item in items:
                consume(item, raw)
        except Exception as error:
            if fail_fast:
                report(error)
            for item in items:
                recover(item, error)
        finally:
            finish(len(items))

    return worker
";
    common::assert_matches_cpython_marshal(source, "terminal_for_finally_handler_control.py");
}
