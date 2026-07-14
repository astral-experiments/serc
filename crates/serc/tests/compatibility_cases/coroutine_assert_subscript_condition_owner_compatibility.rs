use crate::common;

#[test]
fn matches_cpython_marshal_for_coroutine_assert_subscript_condition_owners() {
    let source = r#"async def ingress_style(client, mock, key, pause):
    response = await client()
    assert response.status == 200
    await pause()
    assert len(mock.calls) == 1
    assert key not in mock.calls[-1][3]
    assert mock.calls[-1][3]["source"] == "ingress"
    assert mock.calls[-1][3]["forwarded_for"]
    assert mock.calls[-1][3]["forwarded_host"]
    assert mock.calls[-1][3]["forwarded_proto"]


async def climate_style(home, pause):
    await pause()
    assert home.calls[-1][0] == "set_cooling_async"
    assert home.calls[-1][1]
    assert len(home.connection.calls) == 1
    await pause()
    assert home.calls[-1][0] == "set_cooling_async"
    assert home.calls[-1][1]
    assert len(home.connection.calls) == 2


async def folded_index_styles(value, factory, pause):
    await pause()
    assert value[-1]
    assert value[~0][1]
    assert value[1 + 2][1]
    assert value[1][1 - 2]
    assert value[(1, 2)][0]
    assert factory()[-1][0]


async def plain_literal_control(value, pause):
    await pause()
    assert value[0]
    assert value[0][1]
    assert value[0][1][2]


async def pre_await_control(value, pause):
    assert value[-1][1]
    await pause()


async def dynamic_index_control(value, index, first, second, pause):
    await pause()
    assert value[-index][0]
    assert value[index()][0]
    assert value[first, second][0]


async def inline_control(value, first, second, values, pause):
    await pause()
    assert value[-1][first and second]
    assert value[-1][[item for item in values if item][0]]


async def protected_try_control(value, pause):
    await pause()
    try:
        assert value[-1][1]
    except ValueError:
        return


async def protected_with_control(value, manager, pause):
    await pause()
    with manager:
        assert value[-1][1]


async def protected_async_with_control(value, manager, pause):
    await pause()
    async with manager:
        assert value[-1][1]


async def finalbody_control(value, pause):
    await pause()
    try:
        await pause()
    finally:
        assert value[-1][1]


async def nested_loop_control(values, flag, pause):
    await pause()
    for value in values:
        assert value[-1][1]
    while flag:
        assert values[-1][1]
        flag = False


def generator_control(value):
    yield value
    assert value[-1][1]


def ordinary_control(value):
    assert value[-1][1]
"#;
    common::assert_matches_cpython_marshal(
        source,
        "coroutine_assert_subscript_condition_owners.py",
    );
}
