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
        worker.join()
    if problem is not None:
        raise RuntimeError("work failed") from problem
    report(token)
    return token
