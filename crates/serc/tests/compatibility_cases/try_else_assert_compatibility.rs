use crate::common;

#[test]
fn matches_cpython_marshal_for_try_else_assertion_exit_joins() {
    let source = r"def direct(value):
    try:
        action()
    except ValueError:
        recover()
    else:
        assert value
    after()

def message(value):
    try:
        action()
    except ValueError:
        recover()
    else:
        prepare()
        assert value, 'bad'
    after()

def is_none(value):
    try:
        action()
    except ValueError:
        recover()
    else:
        assert value is None
    after()

def is_not_none(value):
    try:
        action()
    except ValueError:
        recover()
    else:
        prepare()
        assert value is not None
    after()

def multiple_handlers(value):
    try:
        action()
    except ValueError:
        recover()
    except TypeError:
        recover_type()
    else:
        assert value
    after()

def terminating_handler(value):
    try:
        action()
    except ValueError:
        return
    else:
        assert value
    after()

def protected(manager, value):
    with manager:
        try:
            action()
        except ValueError:
            recover()
        else:
            assert value
    after()

async def coroutine(value):
    try:
        await action()
    except ValueError:
        await recover()
    else:
        assert value is None
    after()

def terminal(value):
    try:
        action()
    except ValueError:
        recover()
    else:
        assert value

def nonfinal(value):
    try:
        action()
    except ValueError:
        recover()
    else:
        assert value
        consume(value)
    after()

def nested_suite(value, flag):
    try:
        action()
    except ValueError:
        recover()
    else:
        if flag:
            assert value
    after()
";
    common::assert_matches_cpython_marshal(source, "try_else_assertion_exit_joins.py");
}

#[test]
fn matches_cpython_assert_try_at_end_of_if_with_following_statement() {
    let source = r"def f(enabled, value, recover, tail):
    if enabled:
        try:
            assert value.ready
        except Exception as error:
            recover(error)
    tail()
";
    common::assert_matches_cpython_marshal(source, "if_tail_try_assert.py");
}

#[test]
fn matches_cpython_assert_try_at_end_of_else_with_following_statement() {
    let source = r"def f(enabled, left, right, work, recover, tail):
    if enabled:
        work()
    else:
        try:
            assert left and right
        except Exception as error:
            recover(error)
    tail()
";
    common::assert_matches_cpython_marshal(source, "else_tail_try_assert_and.py");
}

#[test]
fn retains_cpython_direct_try_assert_following_statement() {
    let source = r"def f(value, recover, tail):
    try:
        assert value.ready
    except Exception as error:
        recover(error)
    tail()
";
    common::assert_matches_cpython_marshal(source, "direct_try_assert_control.py");
}

#[test]
fn retains_cpython_expression_try_at_end_of_if() {
    let source = r"def f(enabled, work, recover, tail):
    if enabled:
        try:
            work()
        except Exception as error:
            recover(error)
    tail()
";
    common::assert_matches_cpython_marshal(source, "if_tail_try_expression_control.py");
}
