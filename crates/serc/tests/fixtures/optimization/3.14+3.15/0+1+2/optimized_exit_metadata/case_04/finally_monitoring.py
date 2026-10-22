def probe(values, ready):
    with manager():
        try:
            assert ready
        finally:
            cleanup()
            if values[0].enabled:
                report()
    after()
