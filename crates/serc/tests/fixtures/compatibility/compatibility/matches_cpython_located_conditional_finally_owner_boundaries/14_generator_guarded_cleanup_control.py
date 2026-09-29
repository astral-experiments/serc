def test(cm, lock, enabled):
    with cm:
        try:
            yield work()
        finally:
            if enabled:
                with lock:
                    collect()
