def keyword_return(manager, lookup, source, context, enabled=False):
    saved = context()
    with manager:
        value = lookup.fetch(source)
        if value is None:
            return None
        if enabled or ready(first=value[0], saved=saved, context=context):
            return value[1]
        return None


def name_effect(manager, value, enabled):
    with manager:
        if value is None:
            return None
        if enabled or ready(value):
            consume(value)
        return value


def attribute_return(manager, value, state):
    with manager:
        if value is None:
            return None
        if state.enabled or ready(value):
            return value
        return None


def not_name_raise(manager, value, enabled):
    with manager:
        if value is None:
            return None
        if not enabled or ready(value):
            raise ValueError(value)
        return value


def not_attribute_return(manager, value, state):
    with manager:
        if value is None:
            return None
        if not state.enabled or ready(value):
            return value
        return None


def compare_return_origin(manager, value, enabled):
    with manager:
        if value is None:
            return None
        if enabled or ready(first=value, second=enabled):
            return value
        return None


def terminal_return_control(manager, value, enabled):
    with manager:
        if value is None:
            return None
        if enabled or ready(value):
            return value


def terminal_raise_control(manager, value, enabled):
    with manager:
        if value is None:
            return None
        if enabled or ready(value):
            raise ValueError(value)


def outside_with_control(value, enabled):
    if value is None:
        return None
    if enabled or ready(value):
        return value
    return None


def no_prior_return_control(manager, value, enabled):
    with manager:
        consume(value)
        if enabled or ready(value):
            return value
        return None
