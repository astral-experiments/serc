def pass_handler_call_gap_name(manager, value):
    with manager:
        try:
            observe(value)
        except RuntimeError:
            pass
        prepared = prepare(value)
        result = prepared or {}
        observe(result)


def effect_handler_call_gap_attribute(manager, source, value):
    with manager:
        try:
            observe(value)
        except RuntimeError:
            prepare(value)
        prepared = prepare(value)
        result = source.value or {}
        observe(result)


def raising_handler_name_gap_call(manager, value):
    with manager:
        try:
            observe(value)
        except RuntimeError:
            raise ValueError
        prepared = value
        result = prepare(prepared) or {}
        observe(result)


def raising_body_effect_gap_subscript(manager, cache, value, enabled):
    with manager:
        try:
            if enabled:
                raise ValueError
        except RuntimeError:
            pass
        observe(value)
        result = cache[value] or {}
        observe(result)


def else_return_two_gaps_call(manager, value):
    with manager:
        try:
            observe(value)
        except RuntimeError:
            pass
        else:
            return value
        prepared = value
        observe(prepared)
        result = prepare(prepared) or {}
        observe(result)


def loop_call_gap_attribute(manager, source, values):
    with manager:
        for value in values:
            try:
                observe(value)
            except RuntimeError:
                prepare(value)
            prepared = prepare(value)
            result = source.value or {}
            observe(result)


def immediate_name_control(manager, value):
    with manager:
        try:
            observe(value)
        except RuntimeError:
            pass
        result = value or {}
        observe(result)


def immediate_call_control(manager, value):
    with manager:
        try:
            observe(value)
        except RuntimeError:
            pass
        result = prepare(value) or {}
        observe(result)


def two_handler_immediate_control(manager, value):
    with manager:
        try:
            observe(value)
        except RuntimeError:
            prepare(value)
        except ValueError:
            observe(value)
        result = value or {}
        observe(result)


async def coroutine_immediate_control(manager, value):
    with manager:
        try:
            observe(value)
        except RuntimeError:
            pass
        result = value or {}
        observe(result)
