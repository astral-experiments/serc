use crate::common;

#[test]
fn matches_cpython_marshal_for_async_generator_finally_owner() {
    let source = r#"from __future__ import annotations

import asyncio

async def stream(source, should_stop):
    async def wait_for_stop() -> None:
        while not await should_stop():
            await asyncio.sleep(0.01)

    stop_task = asyncio.create_task(wait_for_stop())
    pending: asyncio.Future[bytes] | None = None
    try:
        while True:
            pending = asyncio.ensure_future(anext(source))
            done, _ = await asyncio.wait(
                (pending, stop_task),
                return_when=asyncio.FIRST_COMPLETED,
            )
            if stop_task in done:
                pending.cancel()
                await asyncio.gather(pending, return_exceptions=True)
                pending = None
                return
            try:
                chunk = pending.result()
            except StopAsyncIteration:
                return
            finally:
                pending = None
            yield chunk
    finally:
        if pending is not None:
            pending.cancel()
            await asyncio.gather(pending, return_exceptions=True)
        stop_task.cancel()
        await asyncio.gather(stop_task, return_exceptions=True)
        close = getattr(source, "aclose", None)
        if close is not None:
            await close()
"#;
    common::assert_matches_cpython_marshal(source, "async_generator_finally_owner.py");
}
