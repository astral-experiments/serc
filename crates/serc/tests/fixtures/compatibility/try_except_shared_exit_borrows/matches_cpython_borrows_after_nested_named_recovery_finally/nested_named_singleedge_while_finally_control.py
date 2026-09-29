def run(worker, initial, load, report):
    try:
        if worker.offline:
            with load() as items:
                for item in items:
                    report(item)
            return
        for item in initial:
            report(item)
        try:
            while not worker.stop():
                for item in worker.items():
                    report(item)
        except Exception as error:
            report(error)
    finally:
        worker.stopped.set()
