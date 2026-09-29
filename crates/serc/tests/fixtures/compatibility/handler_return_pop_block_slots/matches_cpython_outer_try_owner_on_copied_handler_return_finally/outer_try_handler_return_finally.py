def run():
    try:
        started = clock.read()
        try:
            work()
            return 0
        except Exception:
            failed()
            return 1
        finally:
            duration((clock.read() - started) * 1000)
    except Exception:
        return 1
