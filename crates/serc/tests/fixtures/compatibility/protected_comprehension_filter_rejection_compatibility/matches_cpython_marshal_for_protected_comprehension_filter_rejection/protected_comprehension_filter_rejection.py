
def try_list_call(values, ready):
    try:
        return [value for value in values if ready(value)]
    except Exception:
        return []


def try_set_negated_attribute(values):
    try:
        return {value for value in values if not value.hidden}
    except Exception:
        return set()


def try_dict_named(values, resolve):
    try:
        return {name: value for value in values if (name := resolve(value))}
    except Exception:
        return {}


def handler_list_membership(values, allowed):
    try:
        raise ValueError()
    except Exception:
        return [value for value in values if value in allowed]


def handler_dict_none(values):
    try:
        raise ValueError()
    except Exception:
        return {value: value for value in values if value is not None}


def try_finally_membership(values, allowed, cleanup):
    try:
        return [value for value in values if value in allowed]
    finally:
        cleanup()


def with_list_negated(values, manager, omit):
    with manager:
        return [value for value in values if not omit(value)]


def with_set_comparison(values, manager, missing):
    with manager:
        return {value for value in values if value != missing}


def with_dict_none(values, manager):
    with manager:
        return {value: value for value in values if value is not None}


def try_with_two_filters(values, manager, allowed, ready):
    try:
        with manager:
            return [value for value in values if value in allowed if ready(value)]
    except Exception:
        return []


def with_try_chained(values, manager, lower, upper):
    with manager:
        try:
            return {value for value in values if lower < value < upper}
        except Exception:
            return set()


def with_with_two_generators(values, extra, manager, ready):
    with manager:
        with manager:
            return {
                value: item
                for value in values
                for item in extra
                if ready(value)
            }


def generator_try(values, ready):
    try:
        yield [value for value in values if ready(value)]
    except Exception:
        yield []


async def coroutine_try(values, ready):
    try:
        return [value for value in values if ready(value)]
    except Exception:
        return []
