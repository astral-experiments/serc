async def f(manager, task, release, pool, values):
    with manager:
        try:
            await task.wait()
            if task.failed():
                raise ValueError("failed")
            print(f"count={len(values)} ready={values.count('ready')}")
            print(f"seen={len(values)} total={len(values)}")
            print(f"remaining={len(values)} final={len(values)}")
            print("complete")
        finally:
            release.set()
            if not task.done():
                task.cancel()
                await wait(task, return_exceptions=True)
            await pool.close()
