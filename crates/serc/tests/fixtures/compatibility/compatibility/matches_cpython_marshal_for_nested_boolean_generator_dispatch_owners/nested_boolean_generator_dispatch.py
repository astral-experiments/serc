def assert_all_all(values):
    assert all(all(item.ready for item in row) for row in values)


def assert_all_any(values):
    assert all(any(item.ready for item in row) for row in values)


def assert_any_all(values):
    assert any(all(item.ready for item in row) for row in values)


def assert_any_any(values):
    assert any(any(item.ready for item in row) for row in values)


def returned_nested_all(values):
    return all(all(item.ready for item in row) for row in values)


def returned_nested_any(values):
    return any(any(item.ready for item in row) for row in values)


def assigned_nested_all_any(values):
    result = all(any(item.ready for item in row) for row in values)
    return result


def assigned_nested_any_all(values):
    result = any(all(item.ready for item in row) for row in values)
    return result


async def coroutine_nested_all_any(values):
    return all(any(item.ready for item in row) for row in values)


async def coroutine_nested_any_all(values):
    return any(all(item.ready for item in row) for row in values)


def generator_nested_all(values):
    yield all(all(item.ready for item in row) for row in values)


async def async_generator_nested_any(values):
    yield any(any(item.ready for item in row) for row in values)


def try_protected_nested(values):
    try:
        return all(any(item.ready for item in row) for row in values)
    except RuntimeError:
        return False


def with_protected_nested(values, manager):
    with manager:
        return any(all(item.ready for item in row) for row in values)


def inner_filter_nested(values):
    return all(any(item.ready for item in row if item.visible) for row in values)


def outer_filter_nested(values):
    return any(all(item.ready for item in row) for row in values if row)


def boolean_inner_element_nested(values):
    return all(any(item.ready and item.valid for item in row) for row in values)


def tuple_input_control(values):
    return all(tuple([item.ready for item in row]) for row in values)


def list_comprehension_input_control(values):
    return any([item.ready for item in row] for row in values)


def nonnested_all_control(values):
    return all(item.ready for item in values)


def nonnested_any_control(values):
    return any(item.ready for item in values)


def keyword_call_control(values):
    return all(all(iterable=(item.ready for item in row)) for row in values)


def starred_call_control(values):
    return any(all(*(item.ready for item in row)) for row in values)


def method_call_control(values, checker):
    return all(checker.all(item.ready for item in row) for row in values)


def sum_outer_control(values):
    return sum(any(1 if item.ready else 0 for item in row) for row, _ in values)


def generator_filter_call_control(values):
    return all(row for row in values if any([item.ready for item in row]))


def nested_negated_inner_any_boolean_control(values, enabled):
    return all(enabled and not any(item.ready for item in row) for row in values)


def nested_negated_inner_all_if_control(values, enabled):
    return any((not all(item.ready for item in row)) if enabled else row for row in values)


async def negated_inner_any_coroutine_assert_control(values):
    assert all(not any(item.ready('x') for item in row) for _, row in values)


async def negated_inner_all_coroutine_assert_control(values):
    assert any(not all(item.ready('x') for item in row) for _, row in values)


async def double_negated_inner_any_coroutine_assert_control(values):
    assert all(not not any(item.ready('x') for item in row) for _, row in values)


async def double_negated_inner_all_coroutine_assert_control(values):
    assert any(not not all(item.ready('x') for item in row) for _, row in values)


async def unpacked_inner_any_coroutine_assert_control(values):
    assert all(any(item.ready('x') for item in row) for _, row in values)


async def unpacked_inner_all_coroutine_assert_control(values):
    assert any(all(item.ready('x') for item in row) for _, row in values)


def nested_unpacking_dispatch_control(values):
    return any(all(any(item.ready for item in row) for _, row in group) for group in values)


def parenthesized_nested_all_any(values):
    return all((any((item.ready for item in row)) for row in values))


def multiline_commented_nested_all_any(values):
    return all(
        # outer before
        any(
            # inner before
            item.ready for item in row
            # inner after
        )
        for row in values
        # outer after
    )


def nested_iterable_all_any(values):
    return all(any(item for item in all(value for value in row)) for row in values)


def nested_iterable_any_all(values):
    return any(all(item for item in any(value for value in row)) for row in values)


def ordinary_generator_iterable_control(values):
    return all(sum(item for item in any(value for value in row)) for row in values)


def nested_ordinary_generator_iterable_control(values):
    return any(sum(item for item in all(any(part for part in value) for value in row)) for row in values)


async def coroutine_ordinary_generator_iterable_control(values):
    assert all(sum(item for item in any(value for value in row)) for row in values)


async def asynchronous_generator_iterable_control(values):
    return all(sum(item async for item in any(value for value in row)) for row in values)
