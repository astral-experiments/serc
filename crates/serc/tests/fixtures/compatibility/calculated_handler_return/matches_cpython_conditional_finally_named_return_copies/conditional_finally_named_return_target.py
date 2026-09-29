def retry(engine, dispose):
    try:
        return engine.run()
    except Exception as exc:
        try:
            return engine.retry()
        finally:
            if dispose:
                engine.close()
