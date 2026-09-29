def stream(lock, read_batch, finished, wait):
    while True:
        with lock:
            entries = read_batch()
            if entries:
                pass
            elif finished():
                return
            else:
                wait()
        for entry in entries:
            yield entry
