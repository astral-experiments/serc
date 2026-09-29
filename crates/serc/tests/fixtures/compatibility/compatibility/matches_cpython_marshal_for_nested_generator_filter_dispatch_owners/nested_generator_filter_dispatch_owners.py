def direct_any_filter(items, prefixes):
    return next(
        (item for item in items if any(item.name.startswith(prefix) for prefix in prefixes)),
        None,
    )


def direct_all_filter(items, terms):
    return next(
        (item for item in items if all(term in item.text for term in terms)),
        None,
    )


def negated_any_filter(items, blocked):
    return next((item for item in items if not any(value == item for value in blocked)), None)


def boolean_any_filter(items, enabled, values):
    return tuple(item for item in items if enabled and any(value.ready for value in values))


def call_wrapped_filter_control(items, values):
    return next(
        (item for item in items if accept(any(value == item for value in values))),
        None,
    )


def tuple_target_control(pairs):
    return sorted(label for label, values in pairs if any(value.ready for value in values))


def chained_comparison_control(pairs):
    return sorted(
        label for label, values in pairs if not any(0 <= value < 10 for value in values)
    )


def synchronous_list_control(items, values):
    return [item for item in items if any(value == item for value in values)]
