async def normal_async_for_finally_bool(stream, span):
    try:
        async for item in stream:
            yield item
    finally:
        if span:
            span.finish()


async def normal_async_for_finally_walrus(stream):
    try:
        async for item in stream:
            yield item
    finally:
        if tracker := context.maybe_get():
            tracker.record()


async def normal_and_copied_async_for_finally(stream, span, stop):
    try:
        if stop:
            return
        async for item in stream:
            yield item
    finally:
        if span:
            span.finish()


async def normal_async_for_coroutine_finally(stream, span, consume):
    try:
        async for item in stream:
            await consume(item)
    finally:
        if span:
            span.finish()


async def normal_async_for_nested_finally(stream, span, release):
    try:
        async for item in stream:
            yield item
    finally:
        try:
            if span:
                span.finish()
        finally:
            release()


async def async_for_else_finally(stream, span, completed):
    try:
        async for item in stream:
            yield item
        else:
            completed()
    finally:
        if span:
            span.finish()


async def async_for_break_finally(stream, span, stop):
    try:
        async for item in stream:
            if stop:
                break
            yield item
    finally:
        if span:
            span.finish()


async def async_with_normal_async_for_finally(manager, stream, span):
    async with manager:
        try:
            async for item in stream:
                yield item
        finally:
            if span:
                span.finish()


async def statement_after_async_for_finally(stream, span, after_loop):
    try:
        async for item in stream:
            yield item
        after_loop()
    finally:
        if span:
            span.finish()


async def cleanup_before_async_for_finally_condition(stream, span, cleanup):
    try:
        async for item in stream:
            yield item
    finally:
        cleanup()
        if span:
            span.finish()


async def global_call_async_for_finally_condition(stream, span):
    try:
        async for item in stream:
            yield item
    finally:
        if cleanup_ready(span):
            span.finish()


async def async_for_body_return_finally(stream, span, stop):
    try:
        async for item in stream:
            if stop:
                return
            yield item
    finally:
        if span:
            span.finish()
