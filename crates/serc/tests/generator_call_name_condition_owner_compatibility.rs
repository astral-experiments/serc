#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_generator_call_name_condition_owners() {
    let source = r"def ibis_backtrack_style(value, node_type):
    while isinstance(value, node_type):
        yield value
        value = value.parent


def sympy_approximants_style(items):
    while len(items):
        index = 0
        while items[index] == 0:
            index += 1
        yield items[index]
        items = items[index + 1 :]


def pytest_iter_chain_style(current, request_type):
    while isinstance(current, request_type):
        yield current
        current = current.parent


def zulip_invoices_style(invoices, fetch):
    while len(invoices):
        yield from invoices
        invoices = fetch()


def sklearn_minibatches_style(batch, fetch):
    while len(batch):
        yield batch
        batch = fetch()


def schemathesis_value_style(inner, non_null_type):
    while isinstance(inner, non_null_type):
        inner = inner.of_type
    yield inner


def black_closure_style(value):
    def more_splits_should_be_made():
        return bool(value)

    while more_splits_should_be_made():
        yield value
        value = value[1:]


def global_pair_style(first, second):
    while should_continue(first, second):
        yield first


def global_zero_style():
    while should_continue():
        yield 1


def builtin_zero_styles():
    while locals():
        yield 1
    while globals():
        yield 2
    while super():
        yield 3


def global_yield_argument_style(item, node_type):
    while should_continue((yield item)):
        yield item
    while isinstance((yield item), node_type):
        yield item


def global_yield_from_argument_control(items):
    while should_continue((yield from items)):
        yield items


def global_lambda_default_styles(item, items):
    while should_continue((lambda value=(yield item): value)()):
        yield item
    while should_continue((lambda value=(yield from items): value)()):
        yield items


def global_lambda_body_styles(item, items):
    while should_continue((lambda: (yield item))()):
        yield item
    while should_continue((lambda: (yield from items))()):
        yield items


def global_keyword_style(first, second):
    while should_continue(first=first, second=second):
        yield first


def global_starred_style(items):
    while should_continue(*items):
        yield items


def global_keyword_unpack_style(item, options):
    while should_continue(item, **options):
        yield item


def global_mixed_call_ex_style(item, items, options):
    while should_continue(item, *items, value=item, **options):
        yield item


def global_multiple_unpack_style(item, first, second, left, right):
    while should_continue(*first, item, *second, value=item, **left, **right):
        yield item


def global_large_keyword_style(item):
    while should_continue(
        a00=item,
        a01=item,
        a02=item,
        a03=item,
        a04=item,
        a05=item,
        a06=item,
        a07=item,
        a08=item,
        a09=item,
        a10=item,
        a11=item,
        a12=item,
        a13=item,
        a14=item,
        a15=item,
        a16=item,
        a17=item,
        a18=item,
        a19=item,
        a20=item,
        a21=item,
        a22=item,
        a23=item,
        a24=item,
        a25=item,
        a26=item,
        a27=item,
        a28=item,
        a29=item,
        a30=item,
    ):
        yield item


def global_debug_argument_style(item):
    while should_continue(__debug__, item):
        yield item


def global_nested_keyword_call_style(item):
    while should_continue(nested(value=item)):
        yield item


def global_nested_star_call_style(item, items):
    while should_continue(nested(item, *items)):
        yield item


def global_declared_style(item):
    global should_continue
    while should_continue(item):
        yield item


def local_callable_control(check, item):
    while check(item):
        yield item


def maybe_uninitialized_local_control(ready, item):
    if ready:
        check = should_continue
    while check(item):
        yield item


def deleted_local_control(item):
    check = should_continue
    del check
    while check(item):
        yield item


def captured_local_callable_control(item):
    check = should_continue

    def capture():
        return check

    while check(item):
        yield capture()


def closure_callable_control(check, item):
    def nested():
        while check(item):
            yield item

    return nested()


def nonlocal_callable_control(check, item):
    def nested():
        nonlocal check
        while check(item):
            yield item

    return nested()


class ClassCallableControl:
    def class_cell(self, item):
        while __class__():
            yield item


def nested_tail_control(items):
    for item in items:
        while should_continue(item):
            yield item


def nested_while_tail_control(item):
    while should_continue(item):
        while should_continue(item):
            yield item


def nested_while_else_control(item):
    while should_continue(item):
        yield item
    else:
        while should_continue(item):
            yield item


def inline_chained_compare_control(first, second, third):
    while should_continue(first < second < third):
        yield first


def inline_set_dict_control(first, second, items):
    while should_continue({item for item in items if item}):
        yield first
    while should_continue({item: second for item in items if item}):
        yield first


def ordinary_control(item, consume):
    while should_continue(item):
        item = consume(item)


def direct_method_control(reader, item):
    while reader.ready(item):
        yield item


def named_name_control(next_value, first, second):
    while value := next_value(first, second):
        yield value


def named_attribute_control(reader, first, second):
    while value := reader.read(first, second):
        yield value
";
    common::assert_matches_cpython_marshal(source, "generator_call_name_condition_owners.py");
}
