def f():
    for value in range(10):
        if 2 <= value <= 8:
            print(value)

def constant_false_loop(value):
    while False:
        unreachable()
    return value

def constant_true_break(manager):
    while True:
        try:
            break
        except Exception:
            pass
    manager.value = manager.other

def nested_break_finally():
    while condition:
        try:
            while ():
                if 3:
                    break
        finally:
            return

async def async_tail(items, result):
    async for value in items:
        if value:
            result.append(value)

async def protected_async_for(manager, items):
    with manager:
        async for item in items:
            ...

async def protected_async_comprehension(manager, items):
    async with manager:
        consume({item async for item in items})

async def nested_async_cleanup(items):
    async for item in items:
        await sleep(item)
        async with manager(item):
            consume(item)

def protected_boolean_tail(manager, items):
    with manager:
        for item in items:
            if first(item) and second(item):
                action(item)

def nested_boolean_tail(manager, items):
    with manager:
        for item in items:
            if (first(item) and second(item)) or (third(item) and fourth(item)):
                action(item)

def protected_with_loop(manager, items):
    with manager:
        for item in items:
            while condition:
                if first:
                    action(item)
                elif second:
                    other(item)

def protected_generator_loop(items):
    for item in items:
        while condition:
            if first:
                yield item
            elif second:
                yield item + 1

try:
    for item in items:
        while condition:
            if first:
                action()
            elif second:
                other()
except ValueError:
    handle()
finally:
    cleanup()
