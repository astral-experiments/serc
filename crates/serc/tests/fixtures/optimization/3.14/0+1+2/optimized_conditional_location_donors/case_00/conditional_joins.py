def handler_guard(left, right):
    try:
        work()
    except LookupError:
        if left == right:
            assert left
        else:
            assert right
        finish()


def nested_else_in_loop(manager, items, outer, inner):
    with manager:
        for item in items:
            if outer:
                if inner:
                    left(item)
                else:
                    right(item)
            else:
                assert outer
            finish()


def nested_comparison(value, ready):
    if ready:
        if 0 < value < 10:
            assert ready
        else:
            assert value
    else:
        assert value is None
    finish()


def shared_condition_targets(first, second, outer):
    if outer:
        if first and second:
            assert first
        else:
            assert second
    else:
        assert outer
    if first or second:
        assert first
    else:
        assert second
    finish()


def manager_exit(manager, value):
    with manager:
        if value is None:
            assert value
        else:
            result = consume(value)
            return result


def manager_shared_exit(manager, a, b):
    with manager:
        if a or b:
            assert a
        else:
            return b


def nested_validation(enabled, cached, check, value):
    if enabled:
        if cached:
            assert value
        else:
            prepare(value)
            if check:
                assert value
    else:
        assert enabled
    finish()


def nested_range_validation(enabled, cached, low, value, high):
    if enabled:
        if cached:
            assert value
        else:
            if low < value < high:
                assert value
            assert high
    else:
        assert enabled
    finish()
