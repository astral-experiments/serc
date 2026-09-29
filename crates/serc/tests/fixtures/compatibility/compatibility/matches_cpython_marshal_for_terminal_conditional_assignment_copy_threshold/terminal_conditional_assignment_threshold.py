def direct_name(flag, left, right):
    value = left if flag else right


def direct_attribute(flag, left, right, target):
    target.value = left if flag else right


def subscript(flag, left, right, target):
    target["value"] = left if flag else right


def unpack(flag, left, right):
    first, second = left if flag else right


def chained(flag, left, right):
    first = second = left if flag else right


def complex_attribute(flag, left, right, target):
    target.child.value = left if flag else right


def nested_unpack(flag, left, right):
    [(value,)] = left if flag else right
