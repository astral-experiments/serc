def two_returns(manager, first, second):
    with manager:
        if first:
            return
        if second:
            return None


def leading_assert_raise(manager, value):
    with manager:
        assert value.ready
        if value:
            raise RuntimeError(value)


def leading_loop_return(manager, values, value):
    with manager:
        for candidate in values:
            if candidate:
                value = candidate
        if value:
            return value
    return None


def nested_call_branch_raise(manager, value, fallback):
    with manager:
        if not value.check(fallback or value):
            raise RuntimeError(value)
    return None


def nested_call_branch_return(manager, value, fallback):
    with manager:
        if not value.check(fallback or value):
            return value
    return None


def nested_call_comprehension_return(manager, value, values):
    with manager:
        if not value.check([item for item in values if item]):
            return value
    return None


def leading_consumed_list_comprehension_return(manager, condition, items):
    with manager:
        result = consume([item for item in items])
        if condition:
            return result
    return None


def leading_consumed_set_comprehension_return(manager, condition, items):
    with manager:
        result = consume({item for item in items})
        if condition:
            return result
    return None


def leading_consumed_dict_comprehension_return(manager, condition, items):
    with manager:
        result = consume({item: item for item in items})
        if condition:
            return result
    return None


def leading_finally_name_return(manager, condition, value):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        if condition:
            return value
    return None


def leading_finally_not_attribute_raise(manager, value):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        if not value.ready:
            raise RuntimeError(value)
    return None


def leading_finally_named_non_call_return(manager, condition, value):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        if result := condition:
            return result
    return None


def outside_finally_name_return(manager, condition, value):
    try:
        prepare(value)
    finally:
        cleanup(value)
    with manager:
        if condition:
            return value
    return None


def leading_finally_call_return(manager, value):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        if value.check():
            return value
    return None


def leading_finally_named_call_return(manager, value):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        if result := value.check():
            return result
    return None


def leading_finally_none_return(manager, condition, value):
    with manager:
        try:
            prepare(value)
        finally:
            cleanup(value)
        if condition is None:
            return value
    return None


def nested_call_chained_comparison_return(manager, value, first, second, third):
    with manager:
        if value.check(first < second < third):
            return value
    return None


def nested_named_chained_comparison_return(manager, first, second, third):
    with manager:
        if result := (first < second < third):
            return result
    return None


def nested_fstring_chained_comparison_return(manager, value, first, second, third):
    with manager:
        if value.check(f'{first < second < third}'):
            return value
    return None


def nested_call_lambda_return(manager, value, condition, fallback):
    with manager:
        if not value.check(lambda: condition or fallback):
            return value
    return None


def nested_lambda_default_branch_return(manager, value, condition, fallback):
    with manager:
        if value.check(lambda result=(condition or fallback): result):
            return value
    return None


def nested_lambda_default_chained_comparison_return(manager, value, first, second, third):
    with manager:
        if value.check(lambda result=(first < second < third): result):
            return value
    return None


def nested_lambda_default_comprehension_return(manager, value, values):
    with manager:
        if value.check(lambda result=([item for item in values]): result):
            return value
    return None


def conjunction_control(manager, first, second, value):
    with manager:
        if first and second:
            return value
    return None


def handler_control(manager, condition, value):
    with manager:
        try:
            observe(value)
        except RuntimeError:
            recover(value)
        if condition:
            raise RuntimeError(value)
    return None


async def coroutine_await_control(manager, condition, value):
    with manager:
        await ready(value)
        if condition:
            return value
    return None
