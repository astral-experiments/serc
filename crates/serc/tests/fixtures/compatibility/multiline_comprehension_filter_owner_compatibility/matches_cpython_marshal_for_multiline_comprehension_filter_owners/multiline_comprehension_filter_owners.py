def multiline_simple(values, suffix):
    return [
        item
        for item in values
        if suffix is None or item.name.endswith(suffix)
    ]


def multiline_cast(values, suffix, cast, make):
    return [
        cast("Item", make(name=item.name))
        for item in values
        if suffix is None or item.name.endswith(suffix)
    ]


def multiline_call_element(values, suffix, make):
    return [
        make(item.name, item.color)
        for item in values
        if suffix is None or item.name.endswith(suffix)
    ]


def multiline_name_contains(values, selected):
    return [
        value
        for value in values
        if selected is None or value.kind in selected
    ]


def multiline_name_in(values, selected):
    return [
        value
        for value in values
        if selected is None or selected in value.members
    ]


def multiline_attribute_not_in(values, removed):
    return [
        value
        for value in values
        if value.parent is None or value.parent not in removed
    ]


def multiline_attribute_compare(values, owner, score):
    return [
        value
        for value in values
        if owner.limit is None or score(value) >= owner.limit
    ]


def multiline_attribute_call(values, owner):
    return [
        value
        for value in values
        if owner.limit is None or value.accepts(owner.limit)
    ]


def multiline_nested_and(values, level, names):
    return [
        value
        for value in values
        if (level is None or level == value.level)
        and (names is None or value.name in names)
    ]


def same_line_control(values, suffix):
    return [item for item in values if suffix is None or item.name.endswith(suffix)]


def later_filter_control(values, suffix, ready):
    return [
        item
        for item in values
        if ready(item)
        if suffix is None or item.name.endswith(suffix)
    ]


async def coroutine_control(values, suffix):
    return [
        item
        for item in values
        if suffix is None or item.name.endswith(suffix)
    ]


def yielding_loop_control(values, suffix):
    for item in values:
        if suffix is None or item.name.endswith(suffix):
            yield item
