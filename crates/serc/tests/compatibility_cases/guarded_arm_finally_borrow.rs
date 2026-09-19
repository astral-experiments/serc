use crate::common;

#[test]
fn matches_cpython_marshal_for_synchronous_guarded_arm_finally_borrow() {
    let source = r"from __future__ import annotations

class Refresher:
    @classmethod
    def refresh(cls) -> None:
        source = cls.source
        if source is None:
            raise RuntimeError('missing source')
        interval = cls.interval_for(source)
        try:
            if source == REMOTE:
                payload, interval = cls.fetch()
                if payload:
                    cls.value = cls.decode(payload)
            else:
                cls.value = cls.load()
        except Exception:
            if source == REMOTE:
                cls.token = None
            log.warning('refresh failed', extra={'source': source}, exc_info=True)
        finally:
            cls.next_refresh = cls.clock() + interval
";
    common::assert_matches_cpython_marshal(source, "guarded_arm_finally_borrow.py");
}

#[test]
fn matches_cpython_marshal_for_synchronous_linear_arm_finally_borrow_control() {
    let source = r"from __future__ import annotations

class Refresher:
    @classmethod
    def refresh(cls) -> None:
        source = cls.source
        if source is None:
            raise RuntimeError('missing source')
        interval = cls.interval_for(source)
        try:
            if source == REMOTE:
                payload, interval = cls.fetch()
                cls.value = cls.decode(payload)
            else:
                cls.value = cls.load()
        except Exception:
            if source == REMOTE:
                cls.token = None
            log.warning('refresh failed', extra={'source': source}, exc_info=True)
        finally:
            cls.next_refresh = cls.clock() + interval
";
    common::assert_matches_cpython_marshal(source, "linear_arm_finally_borrow_control.py");
}

#[test]
fn matches_cpython_guarded_compound_finally_entries() {
    for (filename, source) in [
        (
            "coroutine_guarded_compound_finally_entry.py",
            r"async def f(flag, value, closer, first, second, task):
    try:
        if flag:
            return
        value = await task.run()
        if value is not None:
            task.log(value)
            try:
                if flag:
                    await task.work(value)
            finally:
                task.record(value)
    except ValueError:
        task.cancelled()
        raise
    except Exception:
        task.failed()
    finally:
        try:
            if closer is not None:
                try:
                    async with task.timeout():
                        await closer.close()
                except Exception:
                    task.failed()
        finally:
            if first is not None:
                try:
                    await first.close()
                except Exception:
                    task.failed()
            if second is not None:
                try:
                    await second.close()
                except Exception:
                    task.failed()
",
        ),
        (
            "coroutine_shared_guard_compound_finally_control.py",
            r"async def f(flag, value, closer, first, second, task):
    try:
        if flag:
            return
        value = await task.run()
        if flag and value is not None:
            task.log(value)
            try:
                if flag:
                    await task.work(value)
            finally:
                task.record(value)
    except ValueError:
        task.cancelled()
        raise
    except Exception:
        task.failed()
    finally:
        try:
            if closer is not None:
                try:
                    async with task.timeout():
                        await closer.close()
                except Exception:
                    task.failed()
        finally:
            if first is not None:
                try:
                    await first.close()
                except Exception:
                    task.failed()
            if second is not None:
                try:
                    await second.close()
                except Exception:
                    task.failed()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
