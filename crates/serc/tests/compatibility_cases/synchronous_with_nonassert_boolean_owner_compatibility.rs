use crate::common;

#[test]
fn matches_cpython_marshal_for_synchronous_with_nonassert_boolean_owners() {
    let source = r"def sync_assignment_any(manager, values):
    with manager:
        result = any(value.ready for value in values)
    return result


def sync_if_all(manager, values, use):
    with manager:
        if all(value.ready for value in values):
            use(values)


def sync_if_not_any(manager, values, use):
    with manager:
        if not any(value.ready for value in values):
            use(values)


def sync_expression_any(manager, values, use):
    with manager:
        use(any(value.ready for value in values))


def sync_nested_with(manager, nested, values, use):
    with manager:
        with nested:
            result = any(value.ready for value in values)
            use(result)


def sync_try_body(manager, values, use):
    with manager:
        try:
            result = any(value.ready for value in values)
            use(result)
        except ValueError:
            use(None)


def sync_multigen_filtered(manager, entries, use):
    with manager:
        if any(value.ready for entry in entries if entry.enabled
               for value in entry.values if value):
            use(entries)


def sync_nested_comprehension(manager, groups, use):
    with manager:
        result = [any(value.ready for value in group.values) for group in groups]
        use(result)


def sync_generator_resumed(manager, values, use):
    with manager:
        yield values
        if all(value.ready for value in values):
            use(values)


async def sync_coroutine_prior_await(manager, values, tick, use):
    await tick()
    with manager:
        if any(value.ready for value in values):
            use(values)


async def sync_first_iterator_await(manager, tick, use):
    with manager:
        if any(value.ready for value in await tick()):
            use(None)


def sync_shadowed_any(manager, values, use, any):
    with manager:
        if any(value.ready for value in values):
            use(values)


def assertion_control(manager, values):
    with manager:
        assert any(value.ready for value in values)


def tuple_control(manager, values, use):
    with manager:
        result = tuple(value.ready for value in values)
        use(result)


def nested_definition_control(manager, values, use):
    with manager:
        def callback():
            return any(value.ready for value in values)
        use(callback)


def global_any_control(values, use):
    if any(value.ready for value in values):
        use(values)
";
    common::assert_matches_cpython_marshal(source, "synchronous_with_nonassert_boolean_owners.py");
}
