def target(groups, retries, manager, stop):
    try:
        for group in groups:
            for retry in retries:
                try:
                    with manager(group) as values:
                        for value in values:
                            yield value
                            if stop(value):
                                return
                    break
                except Exception:
                    recover(retry)
    finally:
        finish()
