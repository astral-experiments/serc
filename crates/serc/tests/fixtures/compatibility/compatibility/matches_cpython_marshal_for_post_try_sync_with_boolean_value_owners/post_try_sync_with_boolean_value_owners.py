from __future__ import annotations


def first_value_marker(manager, items, consume):
    with manager:
        for item in items:
            try:
                value = item.read()
            except ValueError:
                continue
            result = value.first or value.second
            consume(result)


def three_operands(manager, items, fallback, consume):
    with manager:
        for item in items:
            try:
                value = item.read()
            except ValueError:
                continue
            result = value.get("first") or value.get("second") or fallback(item)
            consume(result)


def keyword_value(manager, items, consume):
    with manager:
        for item in items:
            try:
                value = item.read()
            except ValueError:
                continue
            consume(value=value.first or value.second)


def named_handler(manager, items, consume):
    with manager:
        for item in items:
            try:
                value = item.read()
            except ValueError as error:
                consume(error)
                continue
            result = value.first or value.second
            consume(result)


def else_path(manager, items, consume):
    with manager:
        for item in items:
            try:
                value = item.read()
            except ValueError:
                continue
            else:
                consume(item)
            result = value.first or value.second
            consume(result)


def finally_path(manager, items, consume):
    with manager:
        for item in items:
            try:
                value = item.read()
            finally:
                consume(item)
            result = value.first or value.second
            consume(result)


def outside_loop(manager, item, consume):
    with manager:
        try:
            value = item.read()
        except ValueError:
            return None
        result = value.first or value.second
        consume(result)


async def coroutine_scope(manager, items, consume):
    with manager:
        for item in items:
            try:
                value = item.read()
            except ValueError:
                continue
            result = value.first or value.second
            consume(result)


def generator_scope(manager, items, consume):
    with manager:
        for item in items:
            try:
                value = item.read()
            except ValueError:
                continue
            result = value.first or value.second
            consume(result)
            yield item


def no_try_control(manager, items, consume):
    with manager:
        for item in items:
            result = item.first or item.second
            consume(result)


def statement_barrier_control(manager, items, consume):
    with manager:
        for item in items:
            try:
                value = item.read()
            except ValueError:
                continue
            if value is None:
                continue
            result = value.first or value.second
            consume(result)


def fallthrough_handler_control(manager, items, consume):
    with manager:
        for item in items:
            try:
                value = item.read()
            except ValueError:
                value = item
            result = value.first or value.second
            consume(result)


def reused_owner_call_control(manager, items, consume):
    with manager:
        for item in items:
            try:
                value = item.read()
            except (TypeError, ValueError):
                value = item
            consume(
                consume(
                    first=int(item.first),
                    second=float(item.second),
                    third=int(value.first),
                    fourth=float(value.second),
                    result=value.get("first") or item.get("second"),
                )
            )


def reused_owner_comprehension_control(manager, items, consume):
    with manager:
        for item in items:
            try:
                value = item.read()
            except ValueError:
                continue
            result = consume(
                first=int(item.first),
                second=int(item.second),
                third=float(value.first),
                fourth=float(value.second),
                fifth=int(item.first),
                sixth=float(item.second),
                mapping={key: item for key in (value.mapping or {}).keys()},
            )
            consume(result)


def leading_group_control(manager, items, fallback, consume):
    with manager:
        for item in items:
            try:
                value = item.read()
            except ValueError:
                pass
            if (item.first or item.second) or fallback:
                consume(value)


def outside_with_control(items, consume):
    for item in items:
        try:
            value = item.read()
        except ValueError:
            continue
        result = value.first or value.second
        consume(result)
