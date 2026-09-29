def name_first_no_args(manager, value, enabled):
    with manager:
        if enabled:
            return value
        if value or ready() or fallback():
            observe(value)


def attribute_first_name_arg(manager, value, enabled):
    with manager:
        if enabled:
            return value
        if value.attribute or ready(enabled) or fallback(value):
            observe(value)


def not_name_first_nested_arg(manager, value, enabled):
    with manager:
        if enabled:
            return value
        if (not value) or ready(transform(enabled)) or fallback(value):
            observe(value)


def not_attribute_first_starred(manager, values, enabled):
    with manager:
        if enabled:
            return values
        if (not values.attribute) or ready(*values) or fallback(values):
            observe(values)


def keyword_second_call(manager, value, enabled):
    with manager:
        if enabled:
            return value
        if value or ready(value=enabled) or fallback(value):
            observe(value)


def double_star_second_call(manager, value, enabled, options):
    with manager:
        if enabled:
            return value
        if value or ready(**options) or fallback(value):
            observe(value)


def lambda_argument_second_call(manager, value, enabled):
    with manager:
        if enabled:
            return value
        if value or ready(lambda: enabled) or fallback(value):
            observe(value)


def genexpr_argument_second_call(manager, value, enabled, values):
    with manager:
        if enabled:
            return value
        if value or ready(item for item in values) or fallback(value):
            observe(value)


def two_manager_condition(first, second, value, enabled):
    with first, second:
        if enabled:
            return value
        if (not value) or ready(enabled) or fallback(value):
            observe(value)


def outer_try_condition(manager, value, enabled):
    try:
        with manager:
            if enabled:
                return value
            if value.attribute or ready(enabled) or fallback(value):
                observe(value)
    except LookupError:
        observe(value)


def or_two_control(manager, value, enabled):
    with manager:
        if enabled:
            return value
        if value or ready(enabled):
            return enabled


def or_four_control(manager, value, enabled, flag):
    with manager:
        if enabled:
            return value
        if value or ready(enabled) or flag or fallback(value):
            return enabled


def second_attribute_control(manager, service, value, enabled):
    with manager:
        if enabled:
            return value
        observe(value)
        if value or service.ready(enabled) or fallback(value):
            observe(value)


def call_first_control(manager, value, enabled):
    with manager:
        if enabled:
            return value
        if prepare(value) or ready(enabled) or fallback(value):
            observe(value)


def and_control(manager, value, enabled):
    with manager:
        if enabled:
            return value
        if value and ready(enabled) and fallback(value):
            return enabled


def gap_control(manager, value, enabled):
    with manager:
        if enabled:
            return value
        observe(value)
        if value or ready(enabled) or fallback(value):
            observe(value)


def loop_control(manager, values, value, enabled):
    with manager:
        for item in values:
            if enabled:
                return value
            observe(value)
            if value or ready(item) or fallback(value):
                observe(value)
