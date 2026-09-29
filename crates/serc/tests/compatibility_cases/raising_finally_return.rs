use crate::common;

#[test]
fn matches_cpython_raising_finally_with_branches_and_suspensions() {
    for (returned, cleanup) in [
        ("payload", "raise error"),
        ("produce()", "raise create_error() from cause"),
        ("(lambda: (yield payload))()", "raise error"),
        ("(lambda value=(yield payload): value)()", "raise error"),
        (
            "(lambda value=(first if flag else second): value)()",
            "raise error",
        ),
        ("(yield from items)", "raise error"),
        ("payload", "raise (lambda value=(yield error): value)()"),
        ("payload", "raise error from (yield cause)"),
        ("payload", "raise (error if flag else other)"),
        ("payload", "raise"),
        ("payload", "yield error\n        raise error"),
    ] {
        let source = format!(
            "def f():\n    try:\n        yield item\n        return {returned}\n    finally:\n        {cleanup}\n"
        );
        common::assert_matches_cpython_marshal(&source, "raising_finally_return.py");
    }
}

#[test]
fn raising_finally_runs_after_the_return_value_is_evaluated() {
    common::assert_executes(
        r"
events = []
marker = RuntimeError('cleanup')

def returned():
    events.append('return')
    return object()

def raised():
    events.append('finally')
    return marker

def regular():
    try:
        return returned()
    finally:
        raise raised()

async def coroutine():
    try:
        return returned()
    finally:
        raise raised()

def generator():
    try:
        yield 1
        pass
        return returned()
    finally:
        raise raised()

for action in ('regular', 'coroutine', 'exhaust', 'close', 'throw'):
    events.clear()
    try:
        if action == 'regular':
            regular()
        elif action == 'coroutine':
            coroutine().send(None)
        else:
            value = generator()
            assert next(value) == 1
            if action == 'exhaust':
                next(value)
            elif action == 'close':
                value.close()
            else:
                value.throw(ValueError('original'))
    except RuntimeError as caught:
        assert caught is marker
    else:
        raise AssertionError('cleanup was skipped')
    assert events == (['finally'] if action in ('close', 'throw') else ['return', 'finally'])
print('ok')
",
        "ok\n",
    );
}
