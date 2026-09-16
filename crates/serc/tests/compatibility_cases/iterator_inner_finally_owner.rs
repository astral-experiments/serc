use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_iterator_inner_finally_owner() {
    let source = r"async def run(attempts, pool, lock, retry):
    try:
        for attempt in attempts:
            resource = None
            try:
                resource = await pool.acquire(attempt)
                await work(resource)
                return
            except Stale:
                raise
            except Error as error:
                if retry(error):
                    await pause(attempt)
                    continue
                report(error)
                return
            finally:
                if resource is not None:
                    await pool.release(resource)
    except Stale:
        return
    except Error as error:
        report(error)
    finally:
        async with lock:
            finished()
        await restart()
";
    assert_matches_cpython_marshal(source, "iterator_inner_finally_owner.py");
}

#[test]
fn matches_cpython_while_inner_finally_owner_control() {
    let source = r"async def run(attempts, pool, lock, retry):
    try:
        while attempts:
            attempt = attempts.pop()
            resource = None
            try:
                resource = await pool.acquire(attempt)
                await work(resource)
                return
            except Stale:
                raise
            except Error as error:
                if retry(error):
                    await pause(attempt)
                    continue
                report(error)
                return
            finally:
                if resource is not None:
                    await pool.release(resource)
    except Stale:
        return
    except Error as error:
        report(error)
    finally:
        async with lock:
            finished()
        await restart()
";
    assert_matches_cpython_marshal(source, "while_inner_finally_owner_control.py");
}
