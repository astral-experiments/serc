
def folded_filter_in_with(fields, fallback, manager):
    with manager:
        return [(details.value if details.ready else (name if name is None else fallback)) for name, details in fields for extra in details.values if True]


async def folded_tuple_filter_in_coroutine(fields, fallback):
    return {name: (details.value if details.ready else (name if name is None else fallback)) for name, details in fields if (1,)}
