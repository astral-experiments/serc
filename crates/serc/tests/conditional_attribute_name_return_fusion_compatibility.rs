#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_conditional_attribute_name_return_fusion() {
    let source = r#"def target(choices=None):
    mock = create()
    mock.choices = choices if choices is not None else []
    return mock


def explicit_argument(choices):
    mock = create()
    mock.choices = choices if choices is not None else []
    return mock


def ordinary_condition(choices, enabled):
    mock = create()
    mock.choices = choices if enabled else []
    return mock


def else_name(choices, fallback):
    mock = create()
    mock.choices = choices if choices is not None else fallback
    return mock


def else_tuple(choices):
    mock = create()
    mock.choices = choices if choices is not None else ()
    return mock


def body_call(choices):
    mock = create()
    mock.choices = normalize(choices) if choices is not None else []
    return mock


def else_call(choices):
    mock = create()
    mock.choices = choices if choices is not None else fallback()
    return mock


def else_binary(choices, fallback):
    mock = create()
    mock.choices = choices if choices is not None else fallback + "_copy"
    return mock


def else_boolean(choices, fallback):
    mock = create()
    mock.choices = choices if choices is not None else fallback.choices or ""
    return mock


def subscript(choices):
    mock = create()
    mock["choices"] = choices if choices is not None else []
    return mock


def local(choices):
    result = choices if choices is not None else []
    return result


def following(choices):
    mock = create()
    mock.choices = choices if choices is not None else []
    finish(mock)
    return mock


def no_return(choices):
    mock = create()
    mock.choices = choices if choices is not None else []


def trailing_none(choices):
    mock = create()
    mock.choices = choices if choices is not None else None
    return None


def return_other(choices, other):
    mock = create()
    mock.choices = choices if choices is not None else []
    return other


def generator(choices):
    mock = create()
    mock.choices = choices if choices is not None else []
    yield mock
"#;
    common::assert_matches_cpython_marshal(
        source,
        "conditional_attribute_name_return_fusion_compatibility.py",
    );
}
