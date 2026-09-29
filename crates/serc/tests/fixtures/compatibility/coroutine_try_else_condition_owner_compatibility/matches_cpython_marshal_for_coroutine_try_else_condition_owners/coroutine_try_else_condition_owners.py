async def basic(validate):
    errors = {}
    try:
        result = await validate()
    except ValueError:
        errors["base"] = "invalid"
    else:
        if not result:
            errors["base"] = "empty"
    return errors


async def two_handlers(validate):
    errors = {}
    try:
        result = await validate()
    except ValueError:
        errors["base"] = "invalid"
    except RuntimeError:
        errors["base"] = "failed"
    else:
        if not result:
            errors["base"] = "empty"
    return errors


async def nested_input(user_input, validate):
    errors = {}
    if user_input is not None:
        try:
            result = await validate(user_input)
        except ValueError:
            errors["base"] = "invalid"
        else:
            if not result:
                errors["base"] = "empty"
    return errors


async def positive_test(user_input, validate, observe):
    errors = {}
    if user_input is not None:
        try:
            result = await validate(user_input)
        except ValueError:
            errors["base"] = "invalid"
        else:
            if result:
                observe(result)
    return errors


async def else_follower(user_input, validate, observe):
    errors = {}
    if user_input is not None:
        try:
            result = await validate(user_input)
        except ValueError:
            errors["base"] = "invalid"
        else:
            if not result:
                errors["base"] = "empty"
            observe(result)
    return errors


async def two_awaits(user_input, prepare, validate):
    errors = {}
    if user_input is not None:
        try:
            await prepare()
            result = await validate(user_input)
        except ValueError:
            errors["base"] = "invalid"
        else:
            if not result:
                errors["base"] = "empty"
    return errors


async def bare_await(validate, observe):
    ready = True
    try:
        await validate()
    except ValueError:
        observe(None)
    else:
        if ready:
            observe(ready)
    return ready


async def annotated_await(validate, observe):
    try:
        result: object = await validate()
    except ValueError:
        observe(None)
    else:
        if not result:
            observe(result)
    return result


async def no_await(user_input, validate):
    errors = {}
    if user_input is not None:
        try:
            result = validate(user_input)
        except ValueError:
            errors["base"] = "invalid"
        else:
            if not result:
                errors["base"] = "empty"
    return errors


async def await_before_sync_try(user_input, prepare, validate):
    errors = {}
    await prepare()
    if user_input is not None:
        try:
            result = validate(user_input)
        except ValueError:
            errors["base"] = "invalid"
        else:
            if not result:
                errors["base"] = "empty"
    return errors


async def await_in_handler(user_input, validate, report):
    errors = {}
    if user_input is not None:
        try:
            result = validate(user_input)
        except ValueError:
            await report()
            errors["base"] = "invalid"
        else:
            if not result:
                errors["base"] = "empty"
    return errors


async def async_with_in_try(user_input, manager, validate):
    errors = {}
    if user_input is not None:
        try:
            async with manager:
                result = validate(user_input)
        except ValueError:
            errors["base"] = "invalid"
        else:
            if not result:
                errors["base"] = "empty"
    return errors


async def embedded_await_in_try(value, validate):
    errors = {}
    result = None
    try:
        result = [await validate(value)]
    except ValueError:
        errors["error"] = True
    else:
        if not result:
            errors["empty"] = True
    return errors


async def retained_async_iterator_exit(values, validate):
    errors = {}
    result = None
    try:
        async for item in values:
            result = validate(item)
    except ValueError:
        errors["error"] = True
    else:
        if not result:
            errors["empty"] = True
    return errors


async def terminal_return_body(user_input, validate):
    errors = {}
    try:
        result = await validate(user_input)
        return result
    except ValueError:
        errors["base"] = "invalid"
    else:
        if not result:
            errors["base"] = "empty"
    return errors


async def after_try(user_input, validate):
    errors = {}
    result = None
    if user_input is not None:
        try:
            result = await validate(user_input)
        except ValueError:
            errors["base"] = "invalid"
        if not result:
            errors["base"] = "empty"
    return errors


async def terminal_raise_body(user_input, validate):
    errors = {}
    try:
        result = await validate(user_input)
        raise RuntimeError
    except ValueError:
        errors["base"] = "invalid"
    else:
        if not result:
            errors["base"] = "empty"
    return errors


async def unreachable_await_after_return():
    try:
        return None
        await validate()
    except ValueError:
        recover()
    else:
        if not ready:
            consume()
    observe()


async def unreachable_await_after_raise():
    try:
        raise RuntimeError
        await validate()
    except ValueError:
        recover()
    else:
        if not ready:
            consume()
    observe()


def sync_control(user_input, validate):
    errors = {}
    if user_input is not None:
        try:
            result = validate(user_input)
        except ValueError:
            errors["base"] = "invalid"
        else:
            if not result:
                errors["base"] = "empty"
    return errors
