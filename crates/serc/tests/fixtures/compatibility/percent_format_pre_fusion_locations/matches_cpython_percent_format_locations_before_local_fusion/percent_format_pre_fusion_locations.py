def subscript(mapping, left, right, value):
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
