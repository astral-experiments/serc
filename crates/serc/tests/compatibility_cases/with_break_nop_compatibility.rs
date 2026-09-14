use crate::common;

#[test]
fn matches_cpython_marshal_for_break_after_context_manager() {
    let source = r"def named_retry(manager, value):
    while True:
        before()
        try:
            with manager:
                work()
            break
        except OSError as error:
            if value:
                continue
            report(error)
        return False
    after()
    return True

def unnamed_retry(manager, value):
    while True:
        try:
            with manager:
                work()
            break
        except OSError:
            if value:
                continue
        return False
    after()
    return True

def with_only(manager):
    while True:
        with manager:
            work()
        break
    after()

async def async_with(manager):
    while True:
        async with manager:
            await work()
        break
    after()

def try_only(value):
    while True:
        try:
            work()
            break
        except OSError:
            if value:
                continue
        return False
    after()

def conditional(manager, value):
    while True:
        with manager:
            work()
        if value:
            break
        other()
    after()
";
    common::assert_matches_cpython_marshal(source, "with_break_nop_compatibility.py");
}

#[test]
fn matches_cpython_marshal_for_suppressed_return_loop_backedges() {
    for (filename, source) in [
        (
            "suppressed_return_retry.py",
            r"def retry(managers):
    try:
        for manager in managers:
            with manager:
                return work()
        raise Error
    except Error as error:
        raise Failure from error
",
        ),
        (
            "suppressed_return_retry_window.py",
            r"def retry(managers):
    while True:
        try:
            for manager in managers:
                with manager:
                    work()
                    return result()
            raise Error
        except Error as error:
            if failed(error):
                raise
            report(error)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
