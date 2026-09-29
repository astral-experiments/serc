def minimal(first, second, manager):
    with manager:
        if first:
            return
        result = first or second
        observe(result)


def return_value(first, second, value, manager):
    with manager:
        if first:
            return value
        result = first or second
        observe(result)


def two_returns(first, second, value, manager):
    with manager:
        if first:
            return value
        if second:
            return
        result = first or second
        observe(result)


def prefixed_return(first, second, value, manager):
    with manager:
        observe(value)
        if first:
            observe(value)
            return value
        result = first or second
        observe(result)


def repeated_assignments(first, second, value, manager):
    with manager:
        if first:
            return value
        first_result = first or second
        second_result = second or value
        third_result = value and first
        observe(first_result, second_result, third_result)


def loop_return_then_assignment(first, second, value, values, manager):
    with manager:
        for value in values:
            if first:
                return value
            result = first or second
            observe(result)


def two_managers(first, second, value, first_manager, second_manager):
    with first_manager, second_manager:
        if first:
            return value
        result = first or second
        observe(result)


def nested_managers(first, second, value, first_manager, second_manager):
    with first_manager:
        with second_manager:
            if first:
                return value
            result = first or second
            observe(result)


def with_target(first, second, value, manager):
    with manager as resource:
        if first:
            return value
        result = resource or second
        observe(result)


def annotated_and_three_operand(first, second, value, manager):
    with manager:
        if first:
            return value
        result: object = first or second or value
        observe(result)


def call_operand(first, second, value, manager):
    with manager:
        if first:
            return value
        result = select(value) or second
        observe(result)


def return_boolean(first, second, value, manager):
    with manager:
        if first:
            return value
        return first or second


def expression_boolean(first, second, value, manager):
    with manager:
        if first:
            return value
        observe(first or second)


def tuple_boolean(first, second, value, manager):
    with manager:
        if first:
            return value
        result = (first or second, value)
        observe(result)


def nested_defaults(manager, enabled, first, second):
    with manager:
        if enabled:
            return
        callback = lambda value=first or second: value

        def nested(value=first or second):
            return value

        observe(callback, nested)


def call_argument(manager, enabled, first, second):
    with manager:
        if enabled:
            return
        result = select(first or second)
        observe(result)


def call_keyword(manager, enabled, first, second):
    with manager:
        if enabled:
            return
        result = select(value=first or second)
        observe(result)


def formatted_expression(manager, enabled, first, second):
    with manager:
        if enabled:
            return
        result = f"{first or second}"
        observe(result)


def assignment_expression(manager, enabled, first, second):
    with manager:
        if enabled:
            return
        observe(result := first or second)
        observe(result)


async def coroutine(first, second, value, manager):
    await prepare(value)
    with manager:
        if first:
            return value
        result = first or second
        observe(result)


def generator(first, second, value, manager):
    yield value
    with manager:
        if first:
            return value
        result = first or second
        observe(result)


async def async_generator(first, second, value, manager):
    yield value
    with manager:
        if first:
            return
        result = first or second
        observe(result)


def intervening_if_control(first, second, value, enabled, manager):
    with manager:
        if first:
            return value
        if enabled:
            observe(value)
        result = first or second
        observe(result)


def loop_continue_control(first, second, value, values, manager):
    with manager:
        for value in values:
            if first:
                continue
            result = first or second
            observe(result)


def loop_break_control(first, second, value, values, manager):
    with manager:
        for value in values:
            if first:
                break
            result = first or second
            observe(result)


def preceding_try_control(first, second, value, manager):
    with manager:
        try:
            observe(value)
        except RuntimeError:
            observe(first)
        result = first or second
        observe(result)


def active_try_control(first, second, value, manager):
    with manager:
        try:
            if first:
                observe(value)
            result = first or second
            observe(result)
        except RuntimeError:
            observe(second)


def prior_raise_control(first, second, manager):
    with manager:
        if first:
            raise RuntimeError
        result = first or second
        observe(result)


def prior_effect_control(first, second, manager):
    with manager:
        if first:
            observe(first)
        result = first or second
        observe(result)


def prior_pass_control(first, second, manager):
    with manager:
        if first:
            pass
        result = first or second
        observe(result)


def no_prior_control(first, second, manager):
    with manager:
        result = first or second
        observe(result)


def return_before_with_control(first, second, manager):
    if first:
        return
    with manager:
        result = first or second
        observe(result)


def nested_scope_control(manager, enabled, values, first, second):
    with manager:
        if enabled:
            return
        selected = [value or second for value in values]
        mapped = {value: value or second for value in values}
        callback = lambda: first or second
        observe(selected, mapped, callback)


async def async_with_control(first, second, value, manager):
    async with manager:
        if first:
            return value
        result = first or second
        observe(result)
