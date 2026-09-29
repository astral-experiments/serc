def generator_not_name(values):
    for value in values:
        prepared = value.prepare()
        if not prepared:
            continue
        yield prepared


def generator_not_call(values):
    for value in values:
        prepared = value.prepare()
        if not prepared.ready():
            continue
        yield prepared


def generator_call(values):
    for value in values:
        prepared = value.prepare()
        if prepared.skip():
            continue
        yield prepared


def generator_attribute_prefix(values):
    for value in values:
        prepared = value.prepared
        if not prepared:
            continue
        yield prepared


def generator_fstring_prefix(values):
    for value in values:
        prepared = f'{value}'
        if not prepared.strip():
            continue
        yield prepared


def generator_yield_condition(values):
    for value in values:
        prepared = value.prepare()
        if not (yield prepared):
            continue
        yield prepared


async def coroutine_not_call(values):
    for value in values:
        prepared = value.prepare()
        if not prepared.ready():
            continue
        await observe(prepared)


def generator_yield_from_control(values):
    for value in values:
        prepared = value.prepare()
        if not observe((yield from value.items())):
            continue
        yield prepared


async def coroutine_eager_lambda_await_control(values):
    for value in values:
        prepared = value.prepare()
        if not observe(lambda *, item=(await observe(prepared)): item):
            continue
        await observe(prepared)


def generator_subscript_control(values):
    for value in values:
        prepared = values[value]
        if not prepared.ready():
            continue
        yield prepared


def generator_jinja_global_normalizer(values, first, second):
    extensions = {}
    for extension_name in values:
        extension_name = normalize(extension_name)
        if not extension_name:
            continue
        extensions[import_string(extension_name)] = None
    yield first or second
