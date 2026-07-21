from __future__ import annotations


async def one_assignment(manager, response, tick, touch):
    async with manager:
        await tick()
    values = response.data
    if not values:
        touch(response)
    return values


async def two_assignments(manager, response, tick):
    async with manager:
        await tick()
    state = response.data
    values = state.items
    if values:
        raise ValueError
    return values


async def annotated_assignment(manager, response, tick):
    async with manager:
        await tick()
    values: object = response.data
    if not values:
        return None
    return values


async def augmented_assignment(manager, response, values, tick, touch):
    async with manager:
        await tick()
    values += response.data
    if values:
        touch(values)
    return values


async def call_assignment(manager, response, tick, touch):
    async with manager:
        await tick()
    values = response.data()
    if values.ready:
        touch(values)
    return values


async def method_condition(manager, response, tick, touch):
    async with manager:
        await tick()
    values = response.data
    if values.ready():
        touch(values)
    return values


async def boolean_condition(manager, response, tick, touch):
    async with manager:
        await tick()
    values = response.data
    if values and response.ready:
        touch(values)
    return values


async def assertion_condition(manager, response, tick):
    async with manager:
        await tick()
    values = response.data
    assert values
    return values


async def direct_condition_control(manager, values, tick, touch):
    async with manager:
        await tick()
    if not values:
        touch(values)
    return values


async def comparison_control(manager, response, tick, touch):
    async with manager:
        await tick()
    values = response.data
    if values is None:
        touch(response)
    return values


async def suspension_control(manager, response, tick, touch):
    async with manager:
        await tick()
    values = await tick()
    if not values:
        touch(response)
    return values


def synchronous_control(manager, response, touch):
    with manager:
        touch(response)
    values = response.data
    if not values:
        touch(response)
    return values
