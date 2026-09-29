async def test(proc):
    try:
        await work()
    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.communicate(timeout=5)
            except TimeoutError:
                proc.kill()
                proc.communicate(timeout=5)
