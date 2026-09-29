def retry(engine, dispose):
    try:
        return engine.run()
    except Exception as exc:
        try:
            return None
        finally:
            if dispose:
                engine.close()
