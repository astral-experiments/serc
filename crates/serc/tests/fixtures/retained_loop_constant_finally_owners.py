async def outer_for(value, source):
    for item in source:
        try:
            consume(item)
            return
        finally:
            if value is not None:
                await cleanup(value)


async def outer_while(value, source):
    while source:
        try:
            consume(source)
            return
        finally:
            if value is not None:
                await cleanup(value)


async def no_loop(value, source):
    try:
        consume(source)
        return
    finally:
        if value is not None:
            await cleanup(value)


async def return_none(value, source):
    for item in source:
        try:
            consume(item)
            return None
        finally:
            if value is not None:
                await cleanup(value)


async def return_local(value, source):
    for item in source:
        try:
            consume(item)
            return value
        finally:
            if value is not None:
                await cleanup(value)


async def return_constant(value, source):
    for item in source:
        try:
            consume(item)
            return 1234
        finally:
            if value is not None:
                await cleanup(value)


async def no_await(value, source):
    for item in source:
        try:
            consume(item)
            return
        finally:
            if value is not None:
                cleanup(value)


def sync_for(value, source):
    for item in source:
        try:
            consume(item)
            return
        finally:
            if value is not None:
                cleanup(value)


def generator_for(value, source):
    for item in source:
        try:
            consume(item)
            return
        finally:
            if value is not None:
                yield cleanup(value)


async def is_none(value, source):
    for item in source:
        try:
            consume(item)
            return
        finally:
            if value is None:
                await cleanup(value)


async def truth_condition(value, source):
    for item in source:
        try:
            consume(item)
            return
        finally:
            if value:
                await cleanup(value)


async def call_condition(value, source):
    for item in source:
        try:
            consume(item)
            return
        finally:
            if check(value):
                await cleanup(value)


async def attribute_condition(value, source):
    for item in source:
        try:
            consume(item)
            return
        finally:
            if value.attr is not None:
                await cleanup(value)


async def cleanup_while(value, source):
    for item in source:
        try:
            consume(item)
            return
        finally:
            while value is not None:
                value = await cleanup(value)


async def normal_fallthrough(value, source):
    for item in source:
        try:
            consume(item)
        finally:
            if value is not None:
                await cleanup(value)


async def exception_handler(value, source):
    for item in source:
        try:
            consume(item)
            return
        except ValueError:
            recover(item)
        finally:
            if value is not None:
                await cleanup(value)


async def inner_try(value, source):
    for item in source:
        try:
            try:
                consume(item)
                return
            except ValueError:
                recover(item)
        finally:
            if value is not None:
                await cleanup(value)
