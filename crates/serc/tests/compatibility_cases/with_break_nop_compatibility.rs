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
