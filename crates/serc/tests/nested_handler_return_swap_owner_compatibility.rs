#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_nested_handler_return_swap_owners() {
    let source = r"def nested_attribute(_type):
    try:
        return _type.__qualname__
    except AttributeError:
        try:
            return _type.__name__
        except AttributeError:
            return repr(_type)


def nested_name(elasticsearch):
    try:
        return elasticsearch._async.transport
    except AttributeError:
        try:
            return elasticsearch.transport
        except AttributeError:
            return elasticsearch


def outer_following(flag, value):
    try:
        if flag:
            return value.attr
    except AttributeError:
        try:
            return value.other
        except AttributeError:
            return value
    return None


def single_handler_name_control(value):
    try:
        return value.attr
    except AttributeError:
        return value


def single_handler_call_control(value):
    try:
        return value.attr
    except AttributeError:
        return repr(value)
";
    common::assert_matches_cpython_marshal(source, "nested_handler_return_swap_owners.py");
}
