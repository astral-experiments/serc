def subscript_method_owner(manager, items, limit):
    failed = 0
    count = 0
    with manager:
        for item in items:
            count += 1
            if not item.result()["ok"]:
                failed += 1
            if count % 10 == 0 or count == limit:
                consume(item)
    return failed


def attribute_owner(manager, items, state, limit):
    count = 0
    with manager:
        for item in items:
            count += 1
            if state.enabled:
                consume(item)
            if count % 10 == 0 or count == limit:
                consume(item)
    return count


def negated_attribute_owner(manager, items, state, limit):
    count = 0
    with manager:
        for item in items:
            count += 1
            if not state.enabled:
                consume(item)
            if count % 10 == 0 or count == limit:
                consume(item)
    return count


def name_owner(manager, items, enabled, limit):
    count = 0
    with manager:
        for item in items:
            count += 1
            if enabled:
                consume(item)
            if count % 10 == 0 or count == limit:
                consume(item)
    return count


def negated_name_owner(manager, items, enabled, limit):
    count = 0
    with manager:
        for item in items:
            count += 1
            if not enabled:
                consume(item)
            if count % 10 == 0 or count == limit:
                consume(item)
    return count


def method_name_argument_owner(manager, items, state, limit):
    count = 0
    with manager:
        for item in items:
            count += 1
            if state.ready(item):
                consume(item)
            if count % 10 == 0 or count == limit:
                consume(item)
    return count


def negated_method_name_argument_owner(manager, items, state, limit):
    count = 0
    with manager:
        for item in items:
            count += 1
            if not state.ready(item):
                consume(item)
            if count % 10 == 0 or count == limit:
                consume(item)
    return count


def method_constant_argument_owner(manager, items, state, limit):
    count = 0
    with manager:
        for item in items:
            count += 1
            if state.ready("item"):
                consume(item)
            if count % 10 == 0 or count == limit:
                consume(item)
    return count


def negated_method_subscript_argument_owner(manager, items, state, values, limit):
    count = 0
    with manager:
        for item in items:
            count += 1
            if not state.ready(values["item"]):
                consume(item)
            if count % 10 == 0 or count == limit:
                consume(item)
    return count


def nested_method_owner(manager, items, state, limit):
    count = 0
    with manager:
        for item in items:
            count += 1
            if not state.client.ready():
                consume(item)
            if count % 10 == 0 or count == limit:
                consume(item)
    return count


def ordinary_follower_owner(manager, items, state):
    saved = None
    with manager:
        for item in items:
            saved = item
            if state.ready(item):
                consume(item)
            saved = item
    return saved


def boolean_argument_control(manager, items, state, enabled, limit):
    count = 0
    with manager:
        for item in items:
            count += 1
            if state.ready(enabled or item):
                consume(item)
            if count % 10 == 0 or count == limit:
                consume(item)
    return count


def comprehension_argument_control(manager, items, state, limit):
    count = 0
    with manager:
        for item in items:
            count += 1
            if state.ready([value for value in items if value]):
                consume(item)
            if count % 10 == 0 or count == limit:
                consume(item)
    return count


def boolean_slice_control(manager, items, enabled, limit):
    count = 0
    with manager:
        for item in items:
            count += 1
            if item.result()[enabled or "ok"]:
                consume(item)
            if count % 10 == 0 or count == limit:
                consume(item)
    return count


def while_control(manager, items, state, enabled, limit):
    count = 0
    with manager:
        while enabled:
            count += 1
            if state.ready(items):
                consume(items)
            if count % 10 == 0 or count == limit:
                consume(items)
    return count
