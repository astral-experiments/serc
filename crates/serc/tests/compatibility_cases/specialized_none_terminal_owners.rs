use crate::common;

#[test]
fn specialized_none_in_normal_finally_keeps_compacted_owner() {
    common::assert_matches_cpython_marshal(
        r"async def f(it, queue, count):
    err = None
    try:
        while True:
            try:
                item = await anext(it)
            except StopAsyncIteration:
                break
            await queue.put(item)
    except Exception as error:
        log(error)
        err = error
    finally:
        for _ in range(count):
            try:
                if err is not None:
                    queue.put_nowait(err)
                else:
                    queue.put_nowait(None)
            except QueueFull:
                break
        done()
",
        "5040_producer_none.py",
    );
}

#[test]
fn specialized_none_in_nested_loop_keeps_compacted_owner() {
    common::assert_matches_cpython_marshal(
        r"def f(batches, make, emit):
    for batch in batches:
        for item in batch:
            try:
                source = make(item)
                if source is None:
                    continue
                iterator = source.instructions()
                while iterator.more():
                    instruction = iterator.next()
                    for reference in instruction.refs():
                        try:
                            kind = reference.kind()
                            if kind is not None and kind.is_call():
                                target = reference.target()
                                if target is not None:
                                    emit(item, target)
                        except Exception: pass
            except Exception: continue
",
        "0043_nested_loop_none.py",
    );
}

#[test]
fn specialized_none_preserves_existing_owner_controls() {
    common::assert_matches_cpython_marshal(
        r"async def f(it, queue, count):
    err = None
    try:
        while True:
            try:
                item = await anext(it)
            except StopAsyncIteration:
                break
            await queue.put(item)
    except Exception as error:
        log(error)
        err = error
    finally:
        for _ in range(count):
            try:
                if err:
                    queue.put_nowait(err)
                else:
                    queue.put_nowait(None)
            except QueueFull:
                break
        done()
",
        "5040_producer_truth_control.py",
    );
    common::assert_matches_cpython_marshal(
        r#"async def f(self, source, output):
    async with self.manager():
        if self.task is None:
            self.task = start(output)
        try:
            async for item in source:
                match item:
                    case Audio():
                        self.push(item)
                    case Video():
                        await output.push(item)
        finally:
            if self.task:
                log("cancel")
                await cancel(self.task)
"#,
        "6394_manager_truth_control.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def f(value):
    try:
        await work()
    finally:
        if value is not None:
            await cleanup(value)
",
        "simple_finally_none_control.py",
    );
    common::assert_matches_cpython_marshal(
        r#"def f(spark):
    records = None
    try:
        records = load(spark)
        results = records.collect()
    finally:
        if records is not None:
            records.unpersist()
        spark.stop()
    if len(results) != 1:
        raise ValueError("count")
    return results
"#,
        "5480_sync_finally_none.py",
    );
}
