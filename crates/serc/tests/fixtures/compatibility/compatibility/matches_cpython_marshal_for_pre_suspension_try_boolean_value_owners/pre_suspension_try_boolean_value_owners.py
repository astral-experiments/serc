async def nested_or_after_condition(value):
    if value is not None:
        consume(value)
    try:
        import os as module
        result = int((module.getenv("SETTING") or "").strip() or 10)
    except Exception:
        result = 10


async def inner_or_after_condition(value):
    if value is not None:
        consume(value)
    try:
        import os as module
        result = int(module.getenv("SETTING") or "")
    except Exception:
        result = 10


async def outer_or_after_condition(value):
    if value is not None:
        consume(value)
    try:
        import os as module
        result = int(module.getenv("SETTING").strip() or 10)
    except Exception:
        result = 10


async def nested_or_after_two_conditions(value, other):
    if value is not None:
        consume(value)
    if other is not None:
        consume(other)
    try:
        import os as module
        result = int((module.getenv("SETTING") or "").strip() or 10)
    except Exception:
        result = 10


async def nested_or_after_truthy_condition(value):
    if value:
        consume(value)
    try:
        import os as module
        result = int((module.getenv("SETTING") or "").strip() or 10)
    except Exception:
        result = 10


async def nested_or_after_compare_condition(value):
    if value == 1:
        consume(value)
    try:
        import os as module
        result = int((module.getenv("SETTING") or "").strip() or 10)
    except Exception:
        result = 10


def generator_nested_or_after_condition(value):
    if value is not None:
        consume(value)
    try:
        import os as module
        result = int((module.getenv("SETTING") or "").strip() or 10)
    except Exception:
        result = 10
    yield result


async def nested_or_without_condition_control(value):
    try:
        import os as module
        result = int((module.getenv("SETTING") or "").strip() or 10)
    except Exception:
        result = 10


async def nested_or_after_await_control(value, pause):
    if value is not None:
        consume(value)
    await pause()
    try:
        import os as module
        result = int((module.getenv("SETTING") or "").strip() or 10)
    except Exception:
        result = 10


def synchronous_nested_or_control(value):
    if value is not None:
        consume(value)
    try:
        import os as module
        result = int((module.getenv("SETTING") or "").strip() or 10)
    except Exception:
        result = 10


async def nested_or_outside_try_control(value):
    if value is not None:
        consume(value)
    import os as module
    result = int((module.getenv("SETTING") or "").strip() or 10)


async def nested_or_inside_with_control(manager, value):
    if value is not None:
        consume(value)
    with manager:
        import os as module
        result = int((module.getenv("SETTING") or "").strip() or 10)


async def two_call_assignments_control(source, value):
    if value:
        consume(value)
    first = parse(source.get("first"))
    second = parse(source.get("second"))
    try:
        result = parse(source.get("value") or source.get("fallback"))
    except ValueError:
        result = None
    return first, second, result


async def two_annotated_call_assignments_control(source, value):
    if value:
        consume(value)
    first: object = parse(source.get("first"))
    second: object = parse(source.get("second"))
    try:
        result = parse(source.get("value") or source.get("fallback"))
    except ValueError:
        result = None
    return first, second, result


async def separated_call_assignments_control(source, value):
    if not value:
        return None
    first = parse(source.get("first"))
    second = parse(source.get("second"))
    result = None
    try:
        key = getattr(source, "key", "") or ""
        result = await fetch(key)
    except Exception:
        consume(source)
    return first, second, result


async def except_finally_control(source, value):
    if value:
        consume(value)
    try:
        result = parse(source.get("value") or source.get("fallback"))
    except ValueError:
        result = None
    finally:
        consume(value)
    return result
