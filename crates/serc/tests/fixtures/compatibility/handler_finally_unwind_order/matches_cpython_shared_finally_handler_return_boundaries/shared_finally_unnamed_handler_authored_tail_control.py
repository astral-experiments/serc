def recover(engine):
    try:
        return engine.run()
    except Exception:
        try:
            return engine.retry()
        finally:
            try:
                engine.close()
            except OSError:
                pass
            engine.flush()
