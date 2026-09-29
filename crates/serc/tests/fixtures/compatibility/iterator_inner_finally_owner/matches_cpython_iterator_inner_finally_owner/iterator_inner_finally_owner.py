async def run(attempts, pool, lock, retry):
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
