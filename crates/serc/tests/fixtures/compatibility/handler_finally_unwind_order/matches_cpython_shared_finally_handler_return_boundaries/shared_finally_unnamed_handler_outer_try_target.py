def recover(engine):
    try:
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
    except RuntimeError:
        raise
