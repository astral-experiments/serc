async def boolean_filter(groups, ready):
    return [values for values in groups if ready or any(value for value in values)]


async def direct_filter(groups):
    return [values for values in groups if all(value for value in values)]


async def argument_control(groups):
    return [values for values in groups if consume(any(value for value in values))]
