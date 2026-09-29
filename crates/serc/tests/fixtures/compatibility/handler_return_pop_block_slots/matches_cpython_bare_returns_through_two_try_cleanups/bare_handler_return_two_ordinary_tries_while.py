def example():
    try:
        while True:
            try:
                initialize()
                while True:
                    try:
                        message = read()
                        if not message:
                            continue
                        consume(message)
                    except KeyboardInterrupt:
                        record()
                        return
                    except Exception:
                        recover()
            except Exception:
                reset()
    except KeyboardInterrupt:
        finish()
