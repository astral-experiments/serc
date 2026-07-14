#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_synchronous_with_boolean_dispatch_owners() {
    let source = r#"def preceded_assert_any(manager, values, ready):
    with manager:
        assert ready
        assert any(value.ready for value in values)


def preceded_assert_all(manager, values, ready):
    with manager:
        assert ready
        assert all(value.ready for value in values)


def preceded_assert_not_any(manager, values, ready):
    with manager:
        assert ready
        assert not any(value.ready for value in values)


def preceded_assert_message(manager, values, ready):
    with manager:
        assert ready, "not ready"
        assert any(value.ready for value in values), "none ready"


def preceded_filtered(manager, values, ready):
    with manager:
        assert ready
        assert any(value.ready for value in values if value.enabled)


def two_boolean_assertions(manager, values):
    with manager:
        assert all(value.ready for value in values)
        assert any(value.pending for value in values)


def preceded_if(manager, values, flag, prepare):
    with manager:
        if flag:
            prepare(values)
        assert any(value.ready for value in values)


async def preceded_await_any(manager, values, tick):
    with manager:
        await tick()
        assert any(value.ready for value in values)


async def preceded_await_all_message(manager, values, tick):
    with manager:
        await tick()
        assert all(value.ready for value in values), "not ready"


async def preceded_await_not_any_message(manager, values, tick):
    with manager:
        await tick()
        assert not any(value.ready for value in values), "unexpected"


async def async_generator_await(manager, values, tick):
    with manager:
        await tick()
        assert any(value.ready for value in values)
        yield values


def fresh_any(manager, values):
    with manager:
        assert any(value.ready for value in values)


def preceding_call_keeps_owner(manager, values, prepare):
    with manager:
        prepare(values)
        assert any(value.ready for value in values)


def dynamic_owner_one_item(manager, values, prepare):
    with manager:
        prepare(values)
        assert values
        copied = [value for value in values if value.enabled]
        assert any(value.ready for value in copied)


def dynamic_owner_two_items(first, second, values, request):
    with first, second:
        response = request(values)
        assert response.status == 200
        assert response.headers
        copied = [value for value in values if value.enabled]
        assert any(value.ready for value in copied)


def protected_try(manager, values, ready, consume):
    with manager:
        try:
            assert ready
            assert any(value.ready for value in values)
        except ValueError:
            consume(values)


def nested_with(first, second, values, ready):
    with first:
        assert ready
        with second:
            assert any(value.ready for value in values)
"#;
    common::assert_matches_cpython_marshal(source, "synchronous_with_boolean_dispatch_owners.py");
}
