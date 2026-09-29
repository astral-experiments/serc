async def any_before_await(items, observe):
    try:
        if any(item.ready for item in items):
            return "ready"
        observe(items)
    except RuntimeError:
        return "failed"
    return None


async def any_after_await(items, source, observe):
    try:
        items = await source()
        if any(item.ready for item in items):
            return "ready"
        observe(items)
    except RuntimeError:
        return "failed"
    return None


async def all_after_await(items, source, observe):
    try:
        await source()
        if all(item.ready for item in items):
            return "ready"
        observe(items)
    except RuntimeError as error:
        return str(error)
    return None


async def call_and_filter(items, source, ready, observe):
    try:
        await source()
        if any(ready(item) for item in items if item.enabled):
            return "ready"
        observe(items)
    except (RuntimeError, ValueError):
        return "failed"
    return None


async def nested_iterators(groups, source, observe):
    try:
        await source()
        if all(item.ready for group in groups for item in group):
            return "ready"
        observe(groups)
    except RuntimeError:
        return "failed"
    return None


async def boolean_branch(items, source, enabled, observe):
    try:
        await source()
        if enabled and any(item.ready for item in items):
            return "ready"
        observe(items)
    except RuntimeError:
        return "failed"
    return None


async def negated_branch(items, source, observe):
    try:
        await source()
        if not all(item.ready for item in items):
            return "pending"
        observe(items)
    except RuntimeError:
        return "failed"
    return None


async def protected_finally(items, source, observe):
    try:
        await source()
        assert any(item.ready for item in items)
        observe(items)
    finally:
        observe(items)
    return None


async def loop_body(groups, observe):
    for items in groups:
        try:
            if any(item.ready for item in items):
                return "ready"
            observe(items)
        except RuntimeError:
            observe(items)
    return None


async def call_argument(items, source, consume):
    try:
        await source()
        consume(any(item.ready for item in items))
    except RuntimeError:
        return False
    return True


async def assertion(items, source):
    try:
        await source()
        assert all(item.ready for item in items)
    except RuntimeError:
        return False
    return True


def sync_control(items, observe):
    try:
        if any(item.ready for item in items):
            return "ready"
        observe(items)
    except RuntimeError:
        return "failed"
    return None


async def tuple_control(items, consume):
    consume(tuple(item.ready for item in items))
    return True


async def with_control(items, manager, observe):
    try:
        with manager:
            if any(item.ready for item in items):
                return "ready"
            observe(items)
    except RuntimeError:
        return "failed"
    return None


async def handler_control(items, observe):
    try:
        raise RuntimeError
    except RuntimeError:
        if any(item.ready for item in items):
            return "ready"
        observe(items)
    return None


async def async_generator_control(items):
    if any(item.ready for item in items):
        yield "ready"
