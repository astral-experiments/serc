def fresh_global(value, manager):
    with manager:
        result = prepare(value) or {}
        observe(result)


def fresh_local(value, manager, callback):
    with manager:
        result = callback(value) or {}
        observe(result)


def fresh_method(value, manager, source):
    with manager:
        result = source.prepare(value) or {}
        observe(result)


def after_if_global(enabled, value, manager):
    with manager:
        if enabled:
            observe(value)
        result = prepare(value) or {}
        observe(result)


def after_if_assignment_global(enabled, value, manager):
    with manager:
        if enabled:
            prepared = value
        result = prepare(value) or {}
        observe(result)


def after_if_pass_global(enabled, value, manager):
    with manager:
        if enabled:
            pass
        result = prepare(value) or {}
        observe(result)


def after_assert_global(enabled, value, manager):
    with manager:
        assert enabled
        result = prepare(value) or {}
        observe(result)


def after_if_local(enabled, value, manager, callback):
    with manager:
        if enabled:
            observe(value)
        result = callback(value) or {}
        observe(result)


def after_if_method(enabled, value, manager, source):
    with manager:
        if enabled:
            observe(value)
        result = source.prepare(value) or {}
        observe(result)


def after_try_global(value, manager):
    with manager:
        try:
            observe(value)
        except RuntimeError:
            observe(value)
        result = prepare(value) or {}
        observe(result)
