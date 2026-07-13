#[allow(dead_code)]
mod common;

#[test]
fn synchronous_with_terminal_condition_owners_match_cpython_marshal() {
    let source = r"def name_is_none_return(manager, old_span):
    with manager:
        if old_span is None:
            return old_span
    return 1


def not_method_call_return(manager, client):
    with manager:
        if not client.request():
            return None
    return 1


def not_call_args_return(manager, command, install_args):
    with manager:
        if not command(setup, install, *install_args):
            return None
    return 1


def name_return(manager, cache_checked):
    with manager:
        if cache_checked:
            return cache_checked
    return None


def name_collection_return(manager, file_list):
    with manager:
        if file_list:
            return file_list
    return None


def not_name_raise(manager, unlock_pool):
    with manager:
        if not unlock_pool:
            raise RuntimeError(unlock_pool)
    return None


def name_is_not_none_return(manager, trial):
    with manager:
        if trial is not None:
            return trial
    return None


def not_attribute_raise(manager, cursor):
    with manager:
        if not cursor.rowcount:
            raise RuntimeError(cursor)
    return None


def named_call_return(manager, process):
    with manager:
        if pid := process.fork():
            return pid
    return None


def not_name_raise_body(manager, grammar):
    with manager:
        if not grammar:
            observe(grammar)
            raise RuntimeError(grammar)
    return None


def not_call_many_args_return(manager, checker, callee, types, kinds, names, mapping):
    with manager:
        if not checker.check(callee, types, kinds, names, mapping, None):
            return None
    return 1


def named_static_call_return(manager, provider, name):
    with manager:
        if python := provider.find(name):
            return python
    return None


def named_keyword_call_return(manager, lookup, cursor, cache_type, label):
    with manager:
        if found_name := lookup(cursor=cursor, key_parts=(cache_type, label)):
            return found_name
    return None


def named_is_none_raise(manager, cursor):
    with manager:
        try:
            cursor.execute()
        except RuntimeError as error:
            raise ValueError(error) from error
        if (identifier_row := cursor.fetchone()) is None:
            raise RuntimeError(identifier_row)
    return identifier_row


def named_keyword_is_none_return(manager, database, cursor, name, tiers):
    with manager:
        if (cached_value := database.get(cursor=cursor, name=name)) is None:
            return None
    for tier in tiers:
        if tier.name == cached_value:
            return tier
    return None


def named_keyword_is_none_return_body(manager, database, cursor, name):
    with manager:
        if (result := database.get(cursor=cursor, name=name)) is None:
            return None
    try:
        raw_data = database.decode(result)
    except ValueError:
        return None
    try:
        return database.load(raw_data)
    except KeyError:
        return None


def not_name_lines_raise(manager, lines):
    with manager:
        if not lines:
            raise RuntimeError(lines)
    return None


def not_name_grammar_raise(manager, grammar):
    with manager:
        if not grammar:
            raise RuntimeError(grammar)
    return None


def prior_call_comprehension_return(manager, condition, items):
    result = consume([item for item in items])
    with manager:
        if condition:
            return result


def earlier_call_comprehension_return(manager, condition, items):
    consume([item for item in items])
    prepare()
    with manager:
        if condition:
            return condition
";

    common::assert_matches_cpython_marshal(source, "synchronous_with_terminal_condition_owner.py");
}

#[test]
fn synchronous_with_terminal_condition_owner_controls_match_cpython_marshal() {
    let source = r"def two_returns(manager, first, second):
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
";

    common::assert_matches_cpython_marshal(
        source,
        "synchronous_with_terminal_condition_owner_controls.py",
    );
}
