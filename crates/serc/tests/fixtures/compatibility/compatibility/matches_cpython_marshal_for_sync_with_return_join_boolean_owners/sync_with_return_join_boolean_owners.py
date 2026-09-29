def nested_return(first, second, value, manager):
    with manager:
        if first:
            if second:
                return value
        result = first or second
        observe(result)


def else_return(first, second, value, manager):
    with manager:
        if first:
            observe(value)
        else:
            return value
        result = first or second
        observe(result)


def elif_return(first, second, value, enabled, manager):
    with manager:
        if first:
            observe(value)
        elif enabled:
            return value
        result = first or second
        observe(result)


def assignment_gap(first, second, value, manager):
    with manager:
        if first:
            return value
        prepared = value
        selected = prepare(prepared)
        result = first or second
        observe(prepared, selected, result)


def expression_gap(first, second, value, manager):
    with manager:
        if first:
            return value
        observe(value)
        observe(second)
        result = first or second
        observe(result)


def annotated_gap(first, second, value, manager):
    with manager:
        if first:
            return value
        prepared: object = value
        result = first or second
        observe(prepared, result)


def augmented_gap(first, second, value, manager):
    with manager:
        if first:
            return value
        value += second
        result = first or second
        observe(result)


def and_assignment(first, second, value, manager):
    with manager:
        if first:
            return value
        result = first and second
        observe(result)


def nested_boolean(first, second, value, manager):
    with manager:
        if first:
            return value
        result = first and (second or value)
        observe(result)


def return_after_gap(first, second, value, manager):
    with manager:
        if first:
            return value
        prepared = prepare(value)
        return first or second


def two_managers(first, second, value, first_manager, second_manager):
    with first_manager, second_manager:
        if first:
            return value
        prepared = value
        observe(prepared)
        result = first and second
        observe(result)


def long_straight_line_gap(first, second, value, manager):
    with manager:
        if first:
            return value
        prepared_0 = value
        prepared_1 = prepared_0
        prepared_2 = prepared_1
        prepared_3 = prepared_2
        prepared_4 = prepared_3
        prepared_5 = prepared_4
        prepared_6 = prepared_5
        prepared_7 = prepared_6
        prepared_8 = prepared_7
        prepared_9 = prepared_8
        prepared_10 = prepared_9
        prepared_11 = prepared_10
        result = first or second
        observe(prepared_11, result)


def generator_yield_gap(first, second, value, manager):
    with manager:
        if first:
            return value
        yield value
        result = first or second
        observe(result)


async def async_generator_yield_gap(first, second, value, manager):
    with manager:
        if first:
            return
        yield value
        result = first or second
        observe(result)


def list_comprehension_gap(first, second, value, values, manager):
    with manager:
        if first:
            return value
        prepared = [item for item in values if item]
        result = first or second
        observe(prepared, result)


def generator_expression_gap(first, second, value, values, manager):
    with manager:
        if first:
            return value
        prepared = (item for item in values if item)
        result = first or second
        observe(prepared, result)


def both_branches_return_control(first, second, value, manager):
    with manager:
        if first:
            return value
        else:
            return second
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


def for_barrier_control(first, second, value, values, manager):
    with manager:
        if first:
            return value
        for item in values:
            observe(item)
        result = first or second
        observe(result)


def try_barrier_control(first, second, value, manager):
    with manager:
        if first:
            return value
        try:
            prepare(value)
        except RuntimeError:
            observe(value)
        result = first or second
        observe(result)


def with_barrier_control(first, second, value, manager, other_manager):
    with manager:
        if first:
            return value
        with other_manager:
            observe(value)
        result = first or second
        observe(result)


def boolean_gap_control(first, second, value, manager):
    with manager:
        if first:
            return value
        observe(first or second)
        result = first or second
        observe(result)


def named_gap_control(first, second, value, manager):
    with manager:
        if first:
            return value
        observe(prepared := first or second)
        result = first or second
        observe(prepared, result)


def yield_from_control(first, second, value, values, manager):
    with manager:
        if first:
            return value
        yield from values
        result = first or second
        observe(result)


async def await_control(first, second, value, manager):
    with manager:
        if first:
            return value
        await prepare(value)
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


async def async_with_control(first, second, value, manager):
    async with manager:
        if first:
            return value
        prepared = value
        result = first or second
        observe(prepared, result)
