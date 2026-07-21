from __future__ import annotations


def negated_method(manager, first, value, consume):
    with manager:
        if first:
            consume(value)
            return {"status": "first", "value": value}
        if not value.check():
            consume(value)
            return {"status": "second", "value": value}
        consume(value)


def attribute_receiver(manager, first, value, consume):
    with manager:
        if first:
            return {"status": "first", "value": value}
        if not value.state.check():
            consume(value)
            return {"status": "second", "value": value}
        consume(value)


def call_receiver(manager, first, value, consume):
    with manager:
        if first:
            return {"status": "first", "value": value}
        if not value.prepare().check():
            consume(value)
            return {"status": "second", "value": value}
        consume(value)


def positive_method(manager, first, value, consume):
    with manager:
        if first:
            return {"status": "first", "value": value}
        if value.check():
            consume(value)
            return {"status": "second", "value": value}
        consume(value)


def subscript_receiver_control(manager, first, value, key, consume):
    with manager:
        if first:
            return {"status": "first", "value": value}
        if not value[key].check():
            consume(value)
            return {"status": "second", "value": value}
        consume(value)


def attribute_condition_control(manager, first, value, consume):
    with manager:
        if first:
            return {"status": "first", "value": value}
        if not value.ready:
            consume(value)
            return {"status": "second", "value": value}
        consume(value)


def name_condition_control(manager, first, value, consume):
    with manager:
        if first:
            return {"status": "first", "value": value}
        if not value:
            consume(value)
            return {"status": "second", "value": value}
        consume(value)


def raising_branch_control(manager, first, value, consume):
    with manager:
        if first:
            consume(value)
            raise ValueError(value)
        if not value.check():
            consume(value)
            return {"status": "second", "value": value}
        consume(value)


def fallthrough_branch_control(manager, first, value, consume):
    with manager:
        if first:
            consume(value)
        if not value.check():
            consume(value)
            return {"status": "second", "value": value}
        consume(value)


def outside_with_control(first, value, consume):
    if first:
        return {"status": "first", "value": value}
    if not value.check():
        consume(value)
        return {"status": "second", "value": value}
    consume(value)
