from __future__ import annotations


async def post_try_name_condition(action):
    try:
        response = await action(None)
        value = transform(response)
    except RuntimeError:
        return None
    if not value:
        return None
    return True


async def post_try_method_condition(action):
    try:
        response = await action(None)
        value = transform(response)
    except RuntimeError:
        return None
    if not value.ready():
        return None
    return True


async def post_try_paged_condition(client, limit):
    values = []
    for _ in range(limit):
        try:
            response = await client.fetch(limit=10)
            payload = transform(response)
        except ValueError as error:
            return values, False, str(error)
        except RuntimeError as error:
            return values, False, f"runtime: {error}"
        if not payload.get("ready"):
            return values, False, str(payload.get("error") or "missing")
        values.append(payload)
    return values, True, None


async def post_try_attribute_assignment_name_call_control(action, helper, probe):
    try:
        response = await action(None)
        value = helper.transform(response)
    except RuntimeError:
        return None
    if not probe(value):
        return None
    return True


async def post_try_finally_name_call_control(action, helper, probe):
    try:
        value = await action(None)
    finally:
        helper.close()
    if not probe(value):
        return None
    return True


async def post_try_await_only_control(action):
    try:
        value = await action(None)
    except RuntimeError:
        return None
    if not value:
        return None
    return True


async def post_try_local_assignment_control(action):
    try:
        response = await action(None)
        value = action(response)
    except RuntimeError:
        return None
    if not value:
        return None
    return True


def synchronous_control(action):
    try:
        response = action(None)
        value = transform(response)
    except RuntimeError:
        return None
    if not value:
        return None
    return True
