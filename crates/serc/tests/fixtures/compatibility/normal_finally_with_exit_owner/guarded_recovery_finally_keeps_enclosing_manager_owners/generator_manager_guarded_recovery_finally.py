def target(manager, process):
    with manager:
        try:
            yield process
            if process.poll() is not None:
                raise Failure()
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except Timeout:
                    process.kill()
                    process.wait()
