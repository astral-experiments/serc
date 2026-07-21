def generator_name_condition(items, touch):
    for item in items:
        value = item
        if not item:
            continue
        touch(value)
        yield item


def generator_method_condition(items, touch):
    for item in items:
        value = item
        if not item.ready():
            continue
        touch(value)
        yield item


def nested_generator_condition(groups, touch):
    for group in groups:
        for item in group:
            if not item:
                continue
            touch(item)
            yield item


def generator_with_iterator_condition(groups, open_file, touch):
    for group in groups:
        with open_file(group) as values:
            for item in values:
                if not item:
                    continue
                touch(item)
                yield item


async def async_generator_condition(items, touch):
    async for item in items:
        value = item
        if not item:
            continue
        touch(value)
        yield item


async def async_generator_prior_yield_condition(items, touch):
    async for item in items:
        yield touch(item)
        if not item:
            continue
        yield item


def prior_condition_control(items, gate, touch):
    for item in items:
        if gate:
            touch(item)
        if not item:
            continue
        yield item


def prior_none_condition_control(items):
    for item in items:
        if item is None:
            continue
        if not item:
            continue
        yield item


def prior_call_condition_control(items, probe):
    for item in items:
        if not probe(item):
            continue
        if not item:
            continue
        yield item


def none_condition_control(items):
    for item in items:
        value = item
        if item is None:
            continue
        yield value


def boolean_condition_control(items, gate):
    for item in items:
        if item and gate:
            continue
        yield item


def generator_expression_condition_control(groups):
    for group in groups:
        if any(item for item in group):
            continue
        yield group


def generator_yield_from_condition_control(values, observe):
    for value in values:
        prepared = value.prepare()
        if not observe((yield from value.items())):
            continue
        yield prepared


def inline_branch_condition_control(items, gate, touch, probe):
    for item in items:
        if gate:
            touch(item)
        if probe(item if gate else items):
            continue
        yield item


def nested_try_condition_control(items, touch):
    for item in items:
        try:
            if not item:
                continue
            touch(item)
        except RuntimeError:
            touch(None)
        yield item


async def async_generator_suspending_condition_control(items, probe):
    async for item in items:
        if await probe(item):
            continue
        yield item


async def async_iterator_prior_await_control(items, pause, touch):
    async for item in items:
        value = await pause(item)
        if not value:
            continue
        yield touch(value)


async def sync_iterator_prior_await_control(groups, pause, touch):
    for group in groups:
        values = await pause(group)
        if not values:
            continue
        for value in values:
            yield touch(value)


async def async_iterator_prior_await_method_control(items, pause, touch):
    async for item in items:
        value = await pause(item)
        if value.empty():
            continue
        yield touch(value)


def synchronous_control(items, touch):
    for item in items:
        value = item
        if not item:
            continue
        touch(value)


async def coroutine_control(items, pause):
    for item in items:
        if not item:
            continue
        await pause(item)
