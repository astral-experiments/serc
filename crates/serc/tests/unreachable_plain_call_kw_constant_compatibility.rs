#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_unreachable_plain_call_kw_constants() {
    let source = r#"import c_analyzer.datafiles as _datafiles


def imported_module(variables, ignored, root):
    raise NotImplementedError
    _datafiles.write_ignored(variables, ignored, relroot=root)


def plain_name(call, value):
    raise NotImplementedError
    call(value=value)


def two_keywords(call, left, right):
    raise NotImplementedError
    call(left=left.value, right=right.value)


def positional_and_keyword(call, value, other):
    raise NotImplementedError
    call(value.attribute, other=other.attribute)


def generator(call, value):
    yield value
    raise NotImplementedError
    call(value=value)


async def coroutine(call, value):
    await value
    raise NotImplementedError
    call(value=value)


def earlier_literal(call, value):
    raise RuntimeError("earlier")
    call(value=value)


def nested_call_control(call, inner, value):
    raise RuntimeError("earlier")
    call(value=inner(flag=value))


def nested_attribute_control(call, factory, value):
    raise RuntimeError("earlier")
    call(value=factory().attribute)


def starred_control(call, items, value):
    raise RuntimeError("earlier")
    call(*items, value=value)


def unpack_control(call, values, value):
    raise RuntimeError("earlier")
    call(value=value, **values)


def plain_threshold_control(call, value):
    raise RuntimeError("earlier")
    call(value, value, value, value, value, value, value, value,
         value, value, value, value, value, value, value, value,
         value, value, value, value, value, value, value, value,
         value, value, value, value, value=value)


def method_control(target, value):
    raise RuntimeError("earlier")
    target.call(value=value)


def trailing_statement_control(call, value):
    raise RuntimeError("earlier")
    call(value=value)
    call(value=value)
"#;

    common::assert_matches_cpython_marshal(source, "unreachable_plain_call_kw_constants.py");
}
