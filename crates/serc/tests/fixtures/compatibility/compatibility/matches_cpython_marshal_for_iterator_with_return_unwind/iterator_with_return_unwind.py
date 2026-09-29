"""Iterator and context-manager return-unwind compatibility controls."""


def literal_return(items, manager, action):
    for item in items:
        with manager(item):
            action(item)
            return True
    return False


def name_return(items, manager, action, result):
    for item in items:
        with manager(item):
            action(item)
            return result
    return None


def call_return(items, manager, action, result):
    for item in items:
        with manager(item):
            action(item)
            return result(item)
    return None


def tuple_return(items, manager, action, left, right):
    for item in items:
        with manager(item):
            action(item)
            return left, right
    return None


def boolean_return(items, manager, action, left, right):
    for item in items:
        with manager(item):
            action(item)
            return left or right
    return None


def conditional_return(items, manager, action, left, right):
    for item in items:
        with manager(item):
            action(item)
            return left if item else right
    return None


def two_loops(first_items, second_items, manager, action, result):
    for left in first_items:
        for right in second_items:
            with manager(left, right):
                action(left, right)
                return result
    return None


def nested_managers(items, first, second, action, result):
    for item in items:
        with first(item):
            with second(item):
                action(item)
                return result
    return None


def multiple_items(items, first, second, action, result):
    for item in items:
        with first(item), second(item):
            action(item)
            return result
    return None


def bound_items(items, first, second, action):
    for item in items:
        with first(item) as left, second(item) as right:
            return action(left, right)
    return None


def loop_inside_manager(first_items, second_items, manager, action, result):
    for left in first_items:
        with manager(left):
            for right in second_items:
                action(right)
                return result
    return None


def one_handler(items, manager, action, Error):
    for item in items:
        try:
            with manager(item):
                return action(item)
        except Error:
            continue
    return None


def named_handler(items, manager, action, Error):
    for item in items:
        try:
            with manager(item):
                return action(item)
        except Error as error:
            action(error)
    return None


def handler_raise(items, manager, action, Error):
    for item in items:
        try:
            with manager(item):
                return action(item)
        except Error:
            raise
    return None


def two_handlers(items, manager, action, First, Second):
    for item in items:
        try:
            with manager(item):
                return action(item)
        except First:
            continue
        except Second as error:
            action(error)
            return None
    return None


def body_raise(items, manager, action, Error):
    for item in items:
        try:
            with manager(item):
                action(item)
                raise Error
        except Error:
            continue
    return None


def terminal_if(items, manager, action, Error, enabled):
    for item in items:
        try:
            with manager(item):
                action(item)
                if enabled:
                    return True
                raise Error
        except Error:
            continue
    return False


def terminal_while(items, manager, action, Error):
    for item in items:
        try:
            with manager(item):
                action(item)
                while True:
                    continue
        except Error:
            continue
    return False


def generator_return(items, manager, action, result):
    yield None
    for item in items:
        with manager(item):
            action(item)
            return result
    return None


async def sync_for_await(items, manager, action):
    for item in items:
        with manager(item):
            return await action(item)
    return None


async def async_for_await(items, manager, action):
    async for item in items:
        with manager(item):
            return await action(item)
    return None


async def async_manager_sync_for(items, manager, action):
    for item in items:
        async with manager(item):
            return await action(item)
    return None


async def async_manager_async_for(items, manager, action):
    async for item in items:
        async with manager(item):
            return await action(item)
    return None


async def async_generator_return(items, manager, action):
    yield None
    async for item in items:
        with manager(item):
            await action(item)
            return


def manager_outside_loop(items, manager, action, result):
    with manager:
        for item in items:
            action(item)
            return result
    return None


def while_control(manager, action, result):
    while action():
        with manager():
            return result
    return None
