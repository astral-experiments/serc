def example(reader):
    try:
        while True:
            try:
                initialize()
                while True:
                    try:
                        def prompt(value=0) -> str:
                            return "message"
                        message = reader.ask(prompt()).strip()
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
