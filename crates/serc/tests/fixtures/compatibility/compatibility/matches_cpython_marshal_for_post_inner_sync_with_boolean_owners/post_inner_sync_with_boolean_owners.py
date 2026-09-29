def after_inner_pass_global(outer, inner, value):
    with outer:
        with inner:
            pass
        result = prepare(value) or {}
        observe(result)


def after_inner_effect_global(outer, inner, value):
    with outer:
        with inner:
            observe(value)
        result = prepare(value) or {}
        observe(result)


def after_inner_return_global(outer, inner, value):
    with outer:
        with inner:
            pass
        return prepare(value) or {}


def name_control(outer, inner, value):
    with outer:
        with inner:
            pass
        result = value or {}
        observe(result)


def local_call_control(outer, inner, callback, value):
    with outer:
        with inner:
            pass
        result = callback(value) or {}
        observe(result)


def method_call_control(outer, inner, source, value):
    with outer:
        with inner:
            pass
        result = source.prepare(value) or {}
        observe(result)


async def async_inner_control(outer, inner, value):
    with outer:
        async with inner:
            pass
        result = prepare(value) or {}
        observe(result)


def handler_control(outer, inner, value):
    with outer:
        try:
            with inner:
                observe(value)
        except RuntimeError:
            observe(value)
        result = prepare(value) or {}
        observe(result)
