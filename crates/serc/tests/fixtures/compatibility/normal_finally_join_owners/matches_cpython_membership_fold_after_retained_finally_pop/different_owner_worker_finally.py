def f(manager, pool, value, first, second):
    for attempt in range(3):
        try:
            with manager:
                worker = pool.acquire()
                try:
                    try:
                        worker.render(value, first, second)
                    except Error:
                        replacement = pool.replace(worker)
                        pool.release(replacement)
                        raise
                finally:
                    if worker in pool.workers:
                        pool.release(worker)
        except Error:
            retry(attempt)
