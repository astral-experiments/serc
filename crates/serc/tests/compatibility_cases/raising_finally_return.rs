use crate::common;

#[test]
fn matches_cpython_return_through_raising_finally() {
    for source in [
        "def f():\n    try:\n        return 1\n    finally:\n        raise RuntimeError('cleanup')\n",
        "async def f():\n    try:\n        return 1\n    finally:\n        raise RuntimeError('cleanup')\n",
        "def f():\n    try:\n        yield 1\n        pass\n        return 2\n    finally:\n        raise RuntimeError('cleanup')\n",
        "def f():\n    try:\n        yield 1\n        x = 1\n        return x\n    finally:\n        raise RuntimeError('cleanup')\n",
        "def f():\n    try:\n        yield from ()\n        return 2\n    finally:\n        raise RuntimeError('cleanup')\n",
        "def f(value, cleanup, cause):\n    try:\n        return value()\n    finally:\n        record()\n        raise cleanup() from cause\n",
        "def f():\n    try:\n        return\n    finally:\n        raise\n",
        "def f():\n    try:\n        return 1\n    finally:\n        raise RuntimeError('cleanup')\n        pass\n",
        "def f(flag, first, second):\n    try:\n        return 1\n    finally:\n        raise first if flag else second\n",
        "async def f(value, cleanup):\n    try:\n        return await value()\n    finally:\n        raise await cleanup()\n",
        "def f(flag):\n    try:\n        return 1\n    finally:\n        if flag:\n            raise ValueError()\n        else:\n            raise RuntimeError()\n",
        "def f():\n    try:\n        return 1\n    finally:\n        try:\n            record()\n        finally:\n            raise RuntimeError()\n",
    ] {
        common::assert_matches_cpython_marshal(source, "raising_finally_return.py");
    }
}

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
