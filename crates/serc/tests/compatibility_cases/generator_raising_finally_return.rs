use crate::common;

#[test]
fn matches_cpython_generator_return_through_raising_finally() {
    common::assert_matches_cpython_marshal(
        include_str!("../fixtures/generator_raising_finally_return.py"),
        "generator_raising_finally_return.py",
    );
}

#[test]
fn generator_return_evaluates_value_before_raising_finally() {
    common::assert_executes(
        r"
def factory(events, marker):
    def returned():
        events.append('return')
        return object()

    def raised():
        events.append('finally')
        return marker

    def inner():
        try:
            yield 1
            yield 2
            return returned()
        finally:
            raise raised()

    return inner()

for action in ('exhaust', 'close', 'throw'):
    events = []
    marker = RuntimeError('cleanup')
    generator = factory(events, marker)
    assert next(generator) == 1
    try:
        if action == 'exhaust':
            assert next(generator) == 2
            next(generator)
        elif action == 'close':
            generator.close()
        else:
            generator.throw(ValueError('original'))
    except RuntimeError as caught:
        assert caught is marker
    else:
        raise AssertionError('cleanup was skipped')
    assert events == (['return', 'finally'] if action == 'exhaust' else ['finally'])
print('ok')
",
        "ok\n",
    );
}
