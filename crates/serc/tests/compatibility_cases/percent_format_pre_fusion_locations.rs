use crate::common;

#[test]
fn matches_cpython_percent_format_locations_before_local_fusion() {
    common::assert_matches_cpython_marshal(
        r#"def subscript(mapping, left, right, value):
    mapping["prefix%s middle%s end" % (left, right)] = value

def multiline_subscript(mapping, left, right, value):
    mapping[
        "prefix%s middle%s end" % (left, right)
    ] = value

def attribute_target(obj, left, right, value):
    obj.mapping["prefix%s middle%s end" % (left, right)] = value

def call_arguments(callback, first, left, right):
    return callback(first, "prefix%s middle%s end" % (left, right))

def after_store(left, right):
    saved = left
    return "prefix%s end" % (right,)

def without_leading_literal(mapping, left, right, value):
    mapping["%s middle%s end" % (left, right)] = value
"#,
        "percent_format_pre_fusion_locations.py",
    );
}

#[test]
fn matches_cpython_percent_format_locations_after_exception_joins() {
    common::assert_matches_cpython_marshal(
        r#"def recovered_value(value):
    try:
        converted = int(value)
    except ValueError:
        converted = value
    return "(%s %s %%s)" % (value, value), converted
"#,
        "percent_format_exception_join.py",
    );
}

#[test]
fn matches_cpython_percent_format_locations_after_raising_handlers() {
    common::assert_matches_cpython_marshal(
        r#"def process_file(source, value):
    try:
        code = convert(value)
    except ValueError as error:
        raise ValueError('In "%s" loop at %s' % (source, error)) from None
    return '#line 1 "%s"\n%s' % (source, code)


def return_from_handler(value):
    try:
        converted = int(value)
    except ValueError:
        return value
    return "converted=%s" % (converted,)


def conditional_handler(value, recover):
    try:
        converted = int(value)
    except ValueError:
        if recover:
            converted = value
        else:
            raise
    return "converted=%s" % (converted,)
"#,
        "percent_format_raising_handler.py",
    );
}

#[test]
fn matches_cpython_percent_prefix_at_try_entry() {
    for body in [
        "return \"prefix %s-%s\" % (value, value)",
        "return \"prefix %r\" % (value,)",
        "return \"%s-%s\" % (value, value)",
        "return \"prefix %s\" % value",
        "pass\n        return \"prefix %s-%s\" % (value, value)",
    ] {
        let source = format!(
            "def f(value):\n    try:\n        {body}\n    except Exception:\n        return value\n"
        );
        common::assert_matches_cpython_marshal(&source, "percent_prefix_try_entry.py");
    }
    for source in [
        r#"def f(value, flag):
    if flag:
        try:
            return "prefix %s-%s" % (value, value)
        except Exception:
            return value
    return None
"#,
        r#"def f(value):
    try:
        result = "prefix %s-%s" % (value, value)
    finally:
        finish()
    return result
"#,
        r#"def f(value):
    with manager():
        try:
            return "prefix %s-%s" % (value, value)
        except Exception:
            return value
"#,
        r#"async def f(value):
    try:
        return "prefix %s-%s" % (value, value)
    except Exception:
        return value
"#,
        r#"def f(value):
    yield value
    try:
        yield "prefix %s-%s" % (value, value)
    except Exception:
        return value
"#,
    ] {
        common::assert_matches_cpython_marshal(source, "percent_prefix_try_contexts.py");
    }
}
