def target(manager, process):
    with manager:
        try:
            work()
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except Timeout:
                    process.kill()
                    process.wait()
