def finish(work, worker, pending, token, report, full_error):
    problem = None
    try:
        with work() as jobs:
            for job in jobs:
                try:
                    job.run()
                except BaseException as error:
                    problem = error
                    job.stop()
                    break
            if problem is not None:
                for job in jobs:
                    job.cancel()
    finally:
        if worker.is_alive():
            while True:
                try:
                    pending.put(token, timeout=0.5)
                    break
                except full_error:
                    if not worker.is_alive():
                        break
        worker.join()
    if problem is not None:
        raise RuntimeError("work failed") from problem
    report(token)
    return token
