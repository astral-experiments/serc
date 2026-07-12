#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_looped_sync_with_handler_returns() {
    let source = r"def sync_generator(manager, iterator):
    with manager:
        while True:
            try:
                item = next(iterator)
            except StopIteration:
                return
            yield item

def nested_while(manager, iterator):
    with manager:
        while True:
            while True:
                try:
                    item = next(iterator)
                except StopIteration:
                    return
                yield item

def literal_return(manager, iterator):
    with manager:
        while True:
            try:
                item = next(iterator)
            except StopIteration:
                return True
            yield item

def leading_handler_work(manager, iterator):
    with manager:
        while True:
            try:
                item = next(iterator)
            except StopIteration:
                record(None)
                return
            yield item

def two_handlers(manager, iterator):
    with manager:
        while True:
            try:
                item = next(iterator)
            except ValueError:
                continue
            except StopIteration:
                return
            yield item

async def coroutine(manager, iterator):
    with manager:
        while True:
            try:
                await iterator.__anext__()
            except StopAsyncIteration:
                return

async def async_generator(manager, iterator):
    with manager:
        sent = None
        thrown = None
        while True:
            try:
                item = await (iterator.athrow(thrown) if thrown is not None else iterator.asend(sent))
            except StopAsyncIteration:
                return
            try:
                sent, thrown = (yield item), None
            except GeneratorExit:
                await iterator.aclose()
                raise
            except BaseException as exc:
                thrown = exc

def direct_sync_with_control(manager, iterator):
    with manager:
        try:
            item = next(iterator)
        except StopIteration:
            return
        yield item

def no_with_while_control(iterator):
    while True:
        try:
            item = next(iterator)
        except StopIteration:
            return
        yield item

def iterator_continue_control(manager, iterator, values):
    with manager:
        for _ in values:
            try:
                item = next(iterator)
            except StopIteration:
                continue
            yield item

def iterator_break_control(manager, iterator, values):
    with manager:
        for _ in values:
            try:
                item = next(iterator)
            except StopIteration:
                break
            yield item

def nested_with_reraise_control(outer, inner, iterator):
    with outer:
        with inner:
            while True:
                try:
                    item = next(iterator)
                except StopIteration:
                    raise
                yield item

def named_handler_reraise_control(manager, iterator):
    with manager:
        while True:
            try:
                item = next(iterator)
            except StopIteration as exc:
                record(exc)
                raise
            yield item

def name_return_outside_handler_control(manager, iterator, result):
    with manager:
        while True:
            try:
                item = next(iterator)
            except StopIteration:
                break
            yield item
    return result

async def async_with_continue_control(manager, iterator):
    async with manager:
        while True:
            try:
                item = await iterator.__anext__()
            except StopAsyncIteration:
                continue
            yield item
";
    common::assert_matches_cpython_marshal(source, "looped_sync_with_handler_returns.py");
}
