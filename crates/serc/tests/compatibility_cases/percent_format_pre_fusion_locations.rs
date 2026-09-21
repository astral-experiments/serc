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
