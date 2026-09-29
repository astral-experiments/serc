def generator_expression(items):
    return (item or fallback for item in items)

def generator_value(value):
    yield value
    result = value and fallback
    consume(result)

def generator_try(value):
    yield value
    try:
        result = value or fallback
        consume(result)
    finally:
        cleanup()

def ordinary_try(value):
    try:
        result = value or fallback
        consume(result)
    finally:
        cleanup()

def nested_try(value):
    try:
        try:
            result = value and fallback
            consume(result)
        finally:
            inner_cleanup()
    finally:
        outer_cleanup()

def with_only(manager, value):
    with manager:
        result = value or fallback
        consume(result)

def try_inside_with(manager, value):
    with manager:
        try:
            result = value and fallback
            consume(result)
        finally:
            cleanup()

def with_inside_try(manager, value):
    try:
        with manager:
            result = value or fallback
            consume(result)
    finally:
        cleanup()

def comprehension(items):
    return [item and fallback for item in items]

def protected_comprehensions(manager, items):
    try:
        left = [item or fallback for item in items]
        with manager:
            right = {item and fallback for item in items}
        return left, right
    finally:
        cleanup()

def generator_comprehension(items):
    yield [item or fallback for item in items]

async def coroutine_before(value):
    result = value or fallback
    await source
    consume(result)

async def coroutine_after(value):
    await source
    result = value and fallback
    consume(result)

async def coroutine_try(value):
    await source
    try:
        result = value or fallback
        consume(result)
    finally:
        cleanup()

async def coroutine_with(manager, value):
    await source
    with manager:
        result = value and fallback
        consume(result)

async def async_generator(value):
    yield value
    result = value or fallback
    consume(result)

async def async_generator_try(value):
    yield value
    try:
        result = value and fallback
        consume(result)
    finally:
        cleanup()
