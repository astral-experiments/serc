def sync_with_compare(items, manager, selected, limit, action, fallback):
    with manager():
        for item in items:
            if selected(item):
                if item.value < limit:
                    action(item)
            else:
                fallback(item)


def sync_with_boolean(items, manager, selected, first, second, action, fallback):
    with manager():
        for item in items:
            if selected(item):
                if first(item) or second(item):
                    action(item)
            else:
                fallback(item)


def sync_try_compare(items, Error, selected, limit, action, fallback, handle):
    try:
        for item in items:
            if selected(item):
                if item.value < limit:
                    action(item)
            else:
                fallback(item)
    except Error as error:
        handle(error)


def handler_compare(items, Error, selected, limit, action, fallback):
    try:
        action(None)
    except Error:
        for item in items:
            if selected(item):
                if item.value < limit:
                    action(item)
            else:
                fallback(item)


def generator_compare(items, selected, limit, fallback):
    for item in items:
        if selected(item):
            if item.value < limit:
                yield item
        else:
            fallback(item)


def generator_boolean(items, selected, first, second, fallback):
    for item in items:
        if selected(item):
            if first(item) and second(item):
                yield item
        else:
            fallback(item)


async def coroutine_compare(items, selected, limit, action, fallback):
    for item in items:
        if selected(item):
            if item.value < limit:
                await action(item)
        else:
            await fallback(item)


async def coroutine_boolean(items, selected, first, second, action, fallback):
    for item in items:
        if selected(item):
            if first(item) or second(item):
                await action(item)
        else:
            await fallback(item)


async def async_with_compare(items, manager, selected, limit, action, fallback):
    async with manager():
        for item in items:
            if selected(item):
                if item.value < limit:
                    await action(item)
            else:
                await fallback(item)


async def async_generator_compare(items, selected, limit, fallback):
    for item in items:
        if selected(item):
            if item.value < limit:
                yield item
        else:
            await fallback(item)


def folded_with_false(items, manager, selected, action, fallback):
    with manager():
        for item in items:
            if selected(item):
                if False:
                    action(item)
            else:
                fallback(item)


def folded_try_false(items, Error, selected, action, fallback, handle):
    try:
        for item in items:
            if selected(item):
                if False or False:
                    action(item)
            else:
                fallback(item)
    except Error as error:
        handle(error)


def folded_with_and(items, manager, selected, action, fallback):
    with manager():
        for item in items:
            if selected(item):
                if True and False:
                    action(item)
            else:
                fallback(item)


def folded_handler_true(items, Error, selected, action, fallback):
    try:
        action(None)
    except Error:
        for item in items:
            if selected(item):
                if True:
                    action(item)
            else:
                fallback(item)


def folded_handler_debug(items, Error, selected, action, fallback):
    try:
        action(None)
    except Error:
        for item in items:
            if selected(item):
                if __debug__:
                    action(item)
            else:
                fallback(item)


def folded_handler_one(items, Error, selected, action, fallback):
    try:
        action(None)
    except Error:
        for item in items:
            if selected(item):
                if 1:
                    action(item)
            else:
                fallback(item)


def folded_handler_string(items, Error, selected, action, fallback):
    try:
        action(None)
    except Error:
        for item in items:
            if selected(item):
                if "ready":
                    action(item)
            else:
                fallback(item)


def folded_handler_not_false(items, Error, selected, action, fallback):
    try:
        action(None)
    except Error:
        for item in items:
            if selected(item):
                if not False:
                    action(item)
            else:
                fallback(item)
