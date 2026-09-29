def generator_method_and(values, enabled):
    for value in values:
        prepared = value.prepare()
        if enabled and prepared.ready:
            continue
        yield prepared


def generator_strip_or(values):
    for value in values:
        prepared = value.strip()
        if not prepared or prepared.startswith('#'):
            continue
        yield prepared


def generator_yield_first(values):
    for value in values:
        prepared = value.prepare()
        if (yield value) and prepared.ready:
            continue
        yield prepared


def generator_yield_from_later(values, enabled):
    for value in values:
        prepared = value.prepare()
        if enabled and (yield from value.items()):
            continue
        yield prepared


async def coroutine_method_and(values, enabled):
    for value in values:
        prepared = value.prepare()
        if enabled and prepared.ready:
            continue
        await observe(prepared)


def generator_yield_from_first_control(values):
    for value in values:
        prepared = value.prepare()
        if (yield from value.items()) and prepared.ready:
            continue
        yield prepared


def generator_wrapped_yield_from_first_control(values, enabled):
    for value in values:
        prepared = value.prepare()
        if observe((yield from value.items())) and enabled:
            continue
        yield prepared


def generator_fstring_yield_from_first_control(values, enabled):
    for value in values:
        prepared = value.prepare()
        if f'{(yield from value.items())}' and enabled:
            continue
        yield prepared


async def coroutine_eager_lambda_await_first_control(values):
    for value in values:
        prepared = value.prepare()
        if (lambda item=(await observe(prepared)): item) and prepared.ready:
            continue
        await observe(prepared)


async def coroutine_wrapped_eager_lambda_await_first_control(values, enabled):
    for value in values:
        prepared = value.prepare()
        if observe(lambda *, item=(await observe(prepared)): item) and enabled:
            continue
        await observe(prepared)
