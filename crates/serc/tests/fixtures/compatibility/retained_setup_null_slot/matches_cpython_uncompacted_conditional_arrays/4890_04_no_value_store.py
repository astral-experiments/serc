async def f(running, queue):
    try:
        empty = True
        while running.value or not empty:
            try:
                if not running.value:
                    queue.get(timeout=2)
                else:
                    queue.get_nowait()
                empty = False
            except Empty:
                empty = True
                await sleep(0)
    except KeyboardInterrupt:
        pass
