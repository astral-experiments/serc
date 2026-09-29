def f(manager, values, enabled):
    with manager:
        try:
            work()
        finally:
            if enabled:
                for value in values:
                    try:
                        consume(value)
                    except LookupError:
                        break
                else:
                    try:
                        finish()
                    finally:
                        cleanup()
