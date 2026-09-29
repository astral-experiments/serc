def target(manager, process):
    with manager:
        try:
            work()
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=3)
