from __future__ import annotations


async def annotated_or_assignment(source: object, values: list[str] | None = None) -> None:
    if values is None:
        values = []
    initial = first_value(source)
    result = initial + second_value(source) + (values or [])


async def annotated_or_return(source: object, values: list[str] | None = None) -> None:
    if values is None:
        values = []
    initial = first_value(source)
    result = initial + second_value(source) + (values or [])
    return None


async def global_calls_return(source, values):
    if values is None:
        values = []
    initial = first_value(source)
    result = initial + second_value(source) + (values or [])
    return result


async def global_then_local(source, values, build):
    if values is None:
        values = []
    initial = first_value(source)
    result = initial + build(source) + (values or [])
    return result


async def and_value(source, values):
    if values is None:
        values = []
    initial = first_value(source)
    result = initial + second_value(source) + (values and [])
    return result


async def three_operands(source, values, fallback):
    if values is None:
        values = []
    initial = first_value(source)
    result = initial + second_value(source) + (values or fallback or [])
    return result


async def direct_value_control(values, build):
    result = build() + (values or [])
    return result


async def local_then_global_control(source, values, build):
    if values is None:
        values = []
    initial = build(source)
    result = initial + second_value(source) + (values or [])
    return result


async def after_await_control(source, values, wait):
    if values is None:
        values = []
    await wait()
    initial = first_value(source)
    result = initial + second_value(source) + (values or [])
    return result


async def nested_condition_control(source, values):
    if values is None:
        values = []
    initial = first_value(source)
    if (values or source.items) or []:
        consume(initial)
    return None


async def three_slot_owner_control(source, values, build, consume):
    if values is None:
        values = []
    initial = build(source)
    result = consume(
        first=int(source.first),
        second=float(source.second),
        third=int(source.third),
        value=values or [],
    )
    return result


async def four_slot_owner_control(source, values, consume):
    if values is None:
        values = []
    initial = first_value(source)
    result = consume(
        first=int(source.first),
        second=float(source.second),
        third=int(source.third),
        value=values or [],
    )
    return result


async def five_slot_owner_control(source, values, consume):
    if values is None:
        values = []
    initial = first_value(source)
    other = second_value(source)
    result = consume(
        first=int(source.first),
        second=float(source.second),
        third=int(source.third),
        value=values or [],
    )
    return result


def synchronous_control(source, values):
    if values is None:
        values = []
    initial = first_value(source)
    result = initial + second_value(source) + (values or [])
    return result
