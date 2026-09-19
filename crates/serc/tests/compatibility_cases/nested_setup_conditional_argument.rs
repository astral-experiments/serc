use crate::common;

#[test]
fn matches_cpython_marshal_for_nested_try_conditional_call_argument() {
    let source = r#"from __future__ import annotations


async def collect(key, mode, retry=0):
    metrics = services.metrics
    clock = services.clock
    started = clock.now()
    streaming = False
    root = None
    chunks = None
    items = {}
    try:
        store = get_store()
        selected = should_stream(key, mode)
        streaming = selected
        if selected:
            metrics.increment("streaming")
            task = start_decode(key, items)
            root = await store.get_root(key)
            if root is None:
                task.cancel()
                with suppress(Cancelled):
                    await task
                return None
            staged = await task
            chunks = finish_decode(root, staged, items, mode)
        else:
            with measure_read():
                root, blocks = await store.get_tree(key, mode=mode)
            if root is None:
                return None
            record_size(root.content)
            for block in blocks or []:
                record_size(getattr(block, "content", None))
            blocks = select_blocks(root, blocks, key, mode)
            await decode_root(root, items, key)
            if blocks is not None:
                await decode_blocks(blocks, items, key)
            chunks = blocks
    except FirstError:
        if retry < 2:
            metrics.increment("first_retry", tags=[f"mode:{mode}"])
            await pause(0)
            return await collect(key, mode=1, retry=retry + 1)
        metrics.increment("first_failed", tags=[f"mode:{mode}"])
        raise
    except SecondError:
        if retry < 2:
            metrics.increment("second_retry", tags=[f"mode:{mode}"])
            await pause(0)
            return await collect(key, mode=2, retry=retry + 1)
        metrics.increment("second_failed", tags=[f"mode:{mode}"])
        raise
    finally:
        try:
            metrics.distribution(
                "latency",
                value=(clock.now() - started) * 1000,
                tags=[
                    f"mode:{mode}",
                    f"path:{'streaming' if streaming else 'ordinary'}",
                ],
            )
        except Exception:
            pass
    normalize(root, items, key)
    return Result(root=root, chunks=chunks, items=items)
"#;
    common::assert_matches_cpython_marshal(source, "nested_setup_conditional_argument.py");
}
