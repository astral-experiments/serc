def terminal_sum(manager, left, right):
    with manager:
        return convert(left or 0) + convert(right or 0)


def terminal_mapping(manager, state, left, right):
    with manager:
        saved = state.ready(left)
        consume(saved)
        return {"left": convert(left or []), "right": convert(right or [])}


def terminal_tuple(manager, left, right):
    with manager:
        return convert(left or 0), convert(right or 0)


def single_boolean_value(manager, value):
    with manager:
        return convert(value or 0)


def local_callable(manager, convert, left, right):
    with manager:
        return convert(left or 0) + convert(right or 0)


def prior_boolean_then_comprehension(manager, items, first, second, value):
    with manager:
        saved = first or second
        selected = [item for item in items if item]
        consume(saved, selected)
        return convert(value or 0)


def prior_boolean_then_handler(manager, first, second):
    with manager:
        saved = first or second
        try:
            pass
        except global_exception:
            return True
        value = global_factory("value", key=global_other().value())
        global_effect(value, saved)
        return global_call("value") or ""
