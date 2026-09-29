def managed_chunks(manager, finish, ready, timed_out, pause):
    with manager():
        try:
            yield
        finally:
            finish()
            if ready is not None:
                while not ready():
                    if timed_out():
                        break
                    pause()
