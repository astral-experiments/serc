def after_inner_one_gap_call(outer, inner, value):
    with outer:
        with inner:
            pass
        prepared = prepare(value)
        result = prepare(prepared) or {}
        observe(result)


def after_inner_two_gap_name(outer, inner, cache, value):
    with outer:
        with inner:
            observe(value)
        prepared = prepare(value)
        cache[value] = prepared
        return prepared or {"value": value}


def after_branch_one_gap_subscript(manager, cache, value, enabled):
    with manager:
        if enabled:
            observe(value)
        prepared = prepare(value)
        result = cache[prepared] or {}
        observe(result)


def after_branch_two_gap_attribute(manager, cache, source, value, enabled):
    with manager:
        if enabled:
            observe(value)
        else:
            prepare(value)
        prepared = prepare(value)
        cache[value] = prepared
        result = source.value or {}
        observe(result)


def after_assert_one_gap_call(manager, value, enabled):
    with manager:
        assert enabled
        prepared = prepare(value)
        result = prepare(prepared) or {}
        observe(result)


def after_assert_two_gap_name(manager, cache, value, enabled):
    with manager:
        assert enabled
        prepared = prepare(value)
        cache[value] = prepared
        result = prepared or {}
        observe(result)


def one_gap_name_control(manager, value, enabled):
    with manager:
        if enabled:
            observe(value)
        prepared = prepare(value)
        result = prepared or {}
        observe(result)


def two_gap_call_control(manager, value, enabled):
    with manager:
        if enabled:
            observe(value)
        prepared = prepare(value)
        observe(prepared)
        result = prepare(prepared) or {}
        observe(result)


def three_gap_name_control(manager, cache, value, enabled):
    with manager:
        if enabled:
            observe(value)
        prepared = prepare(value)
        observe(prepared)
        cache[value] = prepared
        result = prepared or {}
        observe(result)
