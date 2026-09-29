async def or_assignment(values, flag):
    result = flag or any(value.ready for value in values)
    return result


async def and_assignment(values, flag):
    result = flag and all(value.ready for value in values)
    return result


async def prior_condition(values, flag):
    if flag:
        raise ValueError
    return any(value.ready for value in values)


async def prior_boolean(values, flag, other):
    initial = flag and other
    return initial or any(value.ready for value in values)


async def condition(values, flag):
    if flag or any(value.ready for value in values):
        return True
    return False


async def argument(values, flag, consume):
    return consume(flag, flag or all(value.ready for value in values))


async def conditional(values, flag):
    return any(value.ready for value in values) if flag else True


async def shadowed_callable(values, flag, any):
    return flag or any(value.ready for value in values)


async def awaited_iterable(values, flag):
    return flag or any(value.ready for value in await values())


async def protected_branch(values, flag):
    try:
        return flag or any(value.ready for value in values)
    except ValueError:
        return False


async def first_call_control(values):
    return any(value.ready for value in values)


async def first_operand_control(values, flag):
    return all(value.ready for value in values) and flag
