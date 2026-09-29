def sync_none(items, fetch):
    cursor = None
    while True:
        for item in items:
            item.ready()
        cursor = fetch(cursor)
        if cursor is None:
            break


def sync_equality(items, fetch):
    cursor = None
    while True:
        for item in items:
            item.ready()
        cursor = fetch(cursor)
        if cursor == None:
            break


def sync_truthy(items, fetch):
    cursor = None
    while True:
        for item in items:
            item.ready()
        cursor = fetch(cursor)
        if not cursor:
            break


def sync_boolean(items, first, second):
    while True:
        for item in items:
            item.ready()
        if first and second:
            break


def sync_call(items, ready):
    while True:
        for item in items:
            item.ready()
        if ready():
            break


def sync_generator(items, fetch):
    cursor = None
    while True:
        for item in items:
            yield item
        cursor = fetch(cursor)
        if cursor is None:
            break


def generator_boolean(items, first, second):
    while True:
        for item in items:
            yield item
        if first and second:
            break


async def coroutine_none(items, fetch):
    cursor = None
    while True:
        for item in items:
            item.ready()
        cursor = await fetch(cursor)
        if cursor is None:
            break


async def coroutine_call(items, ready):
    while True:
        for item in items:
            item.ready()
        if await ready():
            break


async def async_generator_none(fetch):
    cursor = None
    while True:
        page = await fetch(cursor)
        for item in page.items:
            yield item
        cursor = page.cursor
        if cursor is None:
            break


async def async_generator_equality(fetch):
    cursor = None
    while True:
        page = await fetch(cursor)
        for item in page.items:
            yield item
        cursor = page.cursor
        if cursor == None:
            break


async def async_generator_truthy(fetch):
    cursor = None
    while True:
        page = await fetch(cursor)
        for item in page.items:
            yield item
        cursor = page.cursor
        if not cursor:
            break


async def async_generator_boolean(fetch, first, second):
    cursor = None
    while True:
        page = await fetch(cursor)
        for item in page.items:
            yield item
        cursor = page.cursor
        if first(cursor) or second(cursor):
            break


def nested_and_earlier_break(first, second, ready, finished):
    while True:
        for group in first:
            for item in second:
                if ready(item):
                    break
                group.record(item)
        if finished():
            break


def earlier_continue(items, ready, finished):
    while True:
        for item in items:
            if not ready(item):
                continue
            item.record()
        if finished():
            break


def earlier_elif(fetch, first, second, finished):
    while True:
        value = fetch()
        if first(value):
            value.record()
        elif second(value):
            value.reset()
        if finished(value):
            break


def earlier_finally(fetch, cleanup, finished):
    while True:
        try:
            value = fetch()
        finally:
            cleanup()
        if finished(value):
            break


def else_control(flag, action):
    while True:
        if flag:
            break
        else:
            action()


def elif_control(first, second, manager):
    with manager():
        while True:
            if first:
                break
            elif second:
                break


def multi_statement_control(flag, action, manager):
    with manager():
        while True:
            if flag:
                action()
                break


def handler_control(action, Error, fetch, finished):
    try:
        action()
    except Error:
        while True:
            value = fetch()
            if finished(value):
                break


def named_handler_control(action, Error, handle, fetch, finished):
    try:
        action()
    except Error as error:
        handle(error)
        while True:
            value = fetch()
            if finished(value):
                break


def finally_control(action, cleanup, flag, value):
    while True:
        try:
            action()
        finally:
            cleanup()
        if flag:
            break
    return value


def with_control(items, manager, finished):
    with manager():
        while True:
            for item in items:
                item.ready()
            if finished():
                break
