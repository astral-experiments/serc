def make_worker(queue):
    failed = None

    def worker() -> None:
        nonlocal failed
        while True:
            signal = queue.get()
            try:
                match signal:
                    case Work(value=value):
                        if failed is not None:
                            continue
                        consume(value)
                    case Stop():
                        break
            except Exception as error:
                failed = error
            finally:
                queue.task_done()

    return worker
