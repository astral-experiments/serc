def return_then_effect_gap(flag, enabled, value, manager):
    with manager:
        if flag:
            return value
        if enabled:
            observe(value)
        result = prepare(value) or {}
        observe(result)


def return_then_assignment_gap(flag, enabled, value, manager):
    with manager:
        if flag:
            return value
        if enabled:
            prepared = value
        result = prepare(value) or {}
        observe(result)


def return_then_else_gap(flag, enabled, value, manager):
    with manager:
        if flag:
            return value
        if enabled:
            observe(value)
        else:
            observe(flag)
        result = prepare(value) or {}
        observe(result)


def return_then_assert_call_gap(flag, enabled, value, manager):
    with manager:
        if flag:
            return value
        assert check(enabled)
        result = prepare(value) or {}
        observe(result)


def no_return_boolean_gap_control(flag, enabled, value, manager):
    with manager:
        if flag:
            observe(flag)
        if enabled and check(value):
            observe(value)
        observe(value)
