use crate::common;

#[test]
fn matches_cpython_marshal_for_awaited_recovery_nested_handler_regression() {
    let source = r"import asyncio

async def stop(self):
    if self.stop_event is None or self.task is None:
        return
    self.stop_event.set()
    try:
        await asyncio.wait_for(self.task, timeout=5)
    except asyncio.TimeoutError:
        self.task.cancel()
        try:
            await self.task
        except asyncio.CancelledError:
            pass
    finally:
        self.stop_event = None
        self.task = None
";
    common::assert_matches_cpython_marshal(source, "awaited_recovery_nested_handler_regression.py");
}
