#[allow(dead_code)]
mod common;

#[test]
fn final_elif_nested_condition_owners_match_cpython_marshal() {
    let source = r"def generator_final_elif_raise(values, stack):
    for value in values:
        if value.first or value.second:
            yield value
        elif value.third:
            if not stack:
                raise RuntimeError(value)
            stack.pop()


def generator_final_elif_name_return(values, stack):
    for value in values:
        if value.first or value.second:
            yield value
        elif value.third:
            if stack:
                return value
            stack.append(value)


def generator_final_elif_nested_else(values, stack):
    for value in values:
        if value.first or value.second:
            yield value
        elif value.third:
            if not stack:
                stack.append(value)
            else:
                stack.pop()


def generator_multiple_elif_nested_continue(values, stack):
    for value in values:
        if value.first or value.second:
            yield value
        elif value.third:
            yield from prepare(value)
        elif value.fourth:
            if not stack:
                continue
            stack.pop()


async def coroutine_final_elif_nested_break(values, stack):
    for value in values:
        if value.first or value.second:
            await observe(value)
        elif value.third:
            if not stack:
                break
            stack.pop()
";

    common::assert_matches_cpython_marshal(source, "final_elif_nested_condition_owners.py");
}

#[test]
fn first_iterator_boolean_condition_owners_match_cpython_marshal() {
    let source = r"def gen_and_continue(values, first, second):
    for value in values:
        if first and second:
            continue
        yield value


def gen_or_continue(values, first, second):
    for value in values:
        if first or second:
            continue
        yield value


def gen_not_call_continue(values, first, checker):
    for value in values:
        if first and not checker(value):
            continue
        yield value


def gen_compare_continue(values, first, allowed):
    for value in values:
        if first and value not in allowed:
            continue
        yield value


def gen_three_continue(values, first, second, third):
    for value in values:
        if first and second and third:
            continue
        yield value


def gen_nested_continue(values, first, second, third):
    for value in values:
        if first and (second or third):
            continue
        yield value


def gen_body_continue(values, first, second):
    for value in values:
        if first and second:
            observe(value)
            continue
        yield value


def gen_else_continue(values, first, second):
    for value in values:
        if first and second:
            continue
        else:
            observe(value)
        yield value


def gen_assign(values, first, second):
    for value in values:
        if first and second:
            value = prepare(value)
        yield value


def gen_assign_else(values, first, second):
    for value in values:
        if first and second:
            value = prepare(value)
        else:
            value = fallback(value)
        yield value


async def coro_and_continue(values, first, second):
    for value in values:
        if first and second:
            continue
        await observe(value)


async def coro_or_continue(values, first, second):
    for value in values:
        if first or second:
            continue
        await observe(value)


async def coro_not_call_continue(values, first, checker):
    for value in values:
        if first and not checker(value):
            continue
        await observe(value)


async def coro_compare_continue(values, first, allowed):
    for value in values:
        if first and value not in allowed:
            continue
        await observe(value)


async def coro_three_continue(values, first, second, third):
    for value in values:
        if first and second and third:
            continue
        await observe(value)


async def coro_nested_continue(values, first, second, third):
    for value in values:
        if first and (second or third):
            continue
        await observe(value)


async def coro_body_continue(values, first, second):
    for value in values:
        if first and second:
            prepare(value)
            continue
        await observe(value)


async def coro_else_continue(values, first, second):
    for value in values:
        if first and second:
            continue
        else:
            prepare(value)
        await observe(value)


async def coro_assign(values, first, second):
    for value in values:
        if first and second:
            value = prepare(value)
        await observe(value)


async def coro_assign_else(values, first, second):
    for value in values:
        if first and second:
            value = prepare(value)
        else:
            value = fallback(value)
        await observe(value)
";

    common::assert_matches_cpython_marshal(source, "iterator_boolean_condition_owner.py");
}

#[test]
fn first_iterator_boolean_condition_owner_controls_match_cpython_marshal() {
    let source = r"def sync_continue(values, first, second):
    for value in values:
        if first and second:
            continue
        observe(value)


def sync_else_continue(values, first, second):
    for value in values:
        if first and second:
            continue
        else:
            prepare(value)
        observe(value)


def generator_value_only(values, first, second):
    for value in values:
        if first and second:
            yield value


async def coroutine_await_only(values, first, second):
    for value in values:
        if first and second:
            await observe(value)


def generator_name_call_prefix(values, first, second):
    for value in values:
        prepared = prepare(value)
        if first and second:
            continue
        yield prepared


async def coroutine_name_call_prefix(values, first, second):
    for value in values:
        prepared = prepare(value)
        if first and second:
            continue
        await observe(prepared)


def generator_nested_condition_prefix(values, first, second):
    for value in values:
        if first:
            observe(value)
        if first and second:
            continue
        yield value


async def coroutine_nested_condition_prefix(values, first, second):
    for value in values:
        if first:
            observe(value)
        if first and second:
            continue
        await observe(value)
";

    common::assert_matches_cpython_marshal(source, "iterator_boolean_condition_owner_controls.py");
}

#[test]
fn first_iterator_boolean_condition_owner_adversarial_cases_match_cpython_marshal() {
    let source = r"def generator_outer_except(values, first, second):
    try:
        for value in values:
            if first and second:
                continue
            yield value
    except RuntimeError:
        recover()


async def coroutine_outer_finally(values, first, second):
    try:
        for value in values:
            if first and second:
                continue
            await observe(value)
    finally:
        finish()


def generator_nested_four_assign_else(values, first, second, third, fourth):
    for value in values:
        if first and (second or third) and fourth:
            value = prepare(value)
        else:
            value = fallback(value)
        yield value


async def coroutine_nested_four_assign_else(values, first, second, third, fourth):
    for value in values:
        if first and (second or third) and fourth:
            value = prepare(value)
        else:
            value = fallback(value)
        await observe(value)


def generator_nested_four_raise(values, first, second, third, fourth):
    for value in values:
        if first and (second or third) and fourth:
            raise RuntimeError(value)
        yield value


async def coroutine_nested_four_return(values, first, second, third, fourth):
    for value in values:
        if first and (second or third) and fourth:
            return value
        await observe(value)


def generator_nested_four_break(values, first, second, third, fourth):
    for value in values:
        if first and (second or third) and fourth:
            break
        yield value


async def coroutine_nested_four_elif(
    values,
    first,
    second,
    third,
    fourth,
    alternative,
):
    for value in values:
        if first and (second or third) and fourth:
            continue
        elif alternative:
            value = prepare(value)
        await observe(value)


def generator_nested_four_for_else(values, first, second, third, fourth):
    for value in values:
        if first and (second or third) and fourth:
            continue
        yield value
    else:
        yield None


async def coroutine_nested_four_for_else(values, first, second, third, fourth):
    for value in values:
        if first and (second or third) and fourth:
            continue
        await observe(value)
    else:
        await observe(None)


def generator_mixed_or_and(values, first, second, third, fourth):
    for value in values:
        if first or (second and third) or fourth:
            continue
        yield value


async def coroutine_mixed_or_and(values, first, second, third, fourth):
    for value in values:
        if first or (second and third) or fourth:
            continue
        await observe(value)


def generator_inner_try(values, first, second):
    for value in values:
        try:
            if first and second:
                continue
            yield value
        except RuntimeError:
            recover()


async def coroutine_inner_try(values, first, second):
    for value in values:
        try:
            if first and second:
                continue
            await observe(value)
        except RuntimeError:
            recover()


def generator_expression_control(values, first, second):
    return (value for value in values if first and second)


def generator_sync_with_control(manager, values, first, second):
    with manager:
        for value in values:
            if first and second:
                continue
            yield value


async def coroutine_sync_with_control(manager, values, first, second):
    with manager:
        for value in values:
            if first and second:
                continue
            await observe(value)
";

    common::assert_matches_cpython_marshal(
        source,
        "iterator_boolean_condition_owner_adversarial.py",
    );
}

#[test]
fn first_iterator_assignment_boolean_condition_owners_match_cpython_marshal() {
    let source = r"def generator_method_and(values, enabled):
    for value in values:
        prepared = value.prepare()
        if enabled and prepared.ready:
            continue
        yield prepared


def generator_strip_or(values):
    for value in values:
        prepared = value.strip()
        if not prepared or prepared.startswith('#'):
            continue
        yield prepared


def generator_yield_first(values):
    for value in values:
        prepared = value.prepare()
        if (yield value) and prepared.ready:
            continue
        yield prepared


def generator_yield_from_later(values, enabled):
    for value in values:
        prepared = value.prepare()
        if enabled and (yield from value.items()):
            continue
        yield prepared


async def coroutine_method_and(values, enabled):
    for value in values:
        prepared = value.prepare()
        if enabled and prepared.ready:
            continue
        await observe(prepared)


def generator_yield_from_first_control(values):
    for value in values:
        prepared = value.prepare()
        if (yield from value.items()) and prepared.ready:
            continue
        yield prepared


def generator_wrapped_yield_from_first_control(values, enabled):
    for value in values:
        prepared = value.prepare()
        if observe((yield from value.items())) and enabled:
            continue
        yield prepared


def generator_fstring_yield_from_first_control(values, enabled):
    for value in values:
        prepared = value.prepare()
        if f'{(yield from value.items())}' and enabled:
            continue
        yield prepared


async def coroutine_eager_lambda_await_first_control(values):
    for value in values:
        prepared = value.prepare()
        if (lambda item=(await observe(prepared)): item) and prepared.ready:
            continue
        await observe(prepared)


async def coroutine_wrapped_eager_lambda_await_first_control(values, enabled):
    for value in values:
        prepared = value.prepare()
        if observe(lambda *, item=(await observe(prepared)): item) and enabled:
            continue
        await observe(prepared)
";

    common::assert_matches_cpython_marshal(
        source,
        "iterator_assignment_boolean_condition_owner.py",
    );
}

#[test]
fn first_iterator_assignment_unary_condition_owners_match_cpython_marshal() {
    let source = r"def generator_not_name(values):
    for value in values:
        prepared = value.prepare()
        if not prepared:
            continue
        yield prepared


def generator_not_call(values):
    for value in values:
        prepared = value.prepare()
        if not prepared.ready():
            continue
        yield prepared


def generator_call(values):
    for value in values:
        prepared = value.prepare()
        if prepared.skip():
            continue
        yield prepared


def generator_attribute_prefix(values):
    for value in values:
        prepared = value.prepared
        if not prepared:
            continue
        yield prepared


def generator_fstring_prefix(values):
    for value in values:
        prepared = f'{value}'
        if not prepared.strip():
            continue
        yield prepared


def generator_yield_condition(values):
    for value in values:
        prepared = value.prepare()
        if not (yield prepared):
            continue
        yield prepared


async def coroutine_not_call(values):
    for value in values:
        prepared = value.prepare()
        if not prepared.ready():
            continue
        await observe(prepared)


def generator_yield_from_control(values):
    for value in values:
        prepared = value.prepare()
        if not observe((yield from value.items())):
            continue
        yield prepared


async def coroutine_eager_lambda_await_control(values):
    for value in values:
        prepared = value.prepare()
        if not observe(lambda *, item=(await observe(prepared)): item):
            continue
        await observe(prepared)


def generator_subscript_control(values):
    for value in values:
        prepared = values[value]
        if not prepared.ready():
            continue
        yield prepared


def generator_jinja_global_normalizer(values, first, second):
    extensions = {}
    for extension_name in values:
        extension_name = normalize(extension_name)
        if not extension_name:
            continue
        extensions[import_string(extension_name)] = None
    yield first or second
";

    common::assert_matches_cpython_marshal(source, "iterator_assignment_unary_condition_owner.py");
}
