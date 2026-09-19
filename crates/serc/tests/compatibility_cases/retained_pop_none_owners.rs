use crate::common;

#[test]
fn retained_pop_none_owner_after_async_manager_body() {
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
            if self.task is not None:
                log("cancel")
                await cancel(self.task)
"#,
        "6394_manager_none.py",
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
}

#[test]
fn retained_pop_none_owner_after_pending_manager_body() {
    common::assert_matches_cpython_marshal(
        r"async def f(state, manager, events, output, log):
    async with manager:
        try:
            async for event in events.consume():
                match event:
                    case int():
                        if state.pending is not None:
                            await state.pending.cancel()
                        state.pending = state.start()
                    case str():
                        if not state.pending:
                            state.pending = state.start()
                        if state.pending:
                            await output.push(state.pending)
                            state.pending = None
        finally:
            if state.pending is not None:
                log()
                await state.pending.cancel()
",
        "6392_manager_none.py",
    );
    common::assert_matches_cpython_marshal(
        r"async def f(state, manager, events, output, log):
    async with manager:
        try:
            async for event in events.consume():
                match event:
                    case int():
                        if state.pending is not None:
                            await state.pending.cancel()
                        state.pending = state.start()
                    case str():
                        if not state.pending:
                            state.pending = state.start()
                        if state.pending:
                            await output.push(state.pending)
                            state.pending = None
        finally:
            if state.pending:
                log()
                await state.pending.cancel()
",
        "6392_manager_truth_control.py",
    );
}

#[test]
fn retained_pop_none_owner_preserves_nested_finally_control() {
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
}
