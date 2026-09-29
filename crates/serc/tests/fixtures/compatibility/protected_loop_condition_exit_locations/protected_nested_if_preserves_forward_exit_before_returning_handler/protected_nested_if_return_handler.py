def probe(groups, manager, build, accept, stop):
    for items in groups:
        with manager():
            for item in items:
                try:
                    value = build(item)
                    if accept(value):
                        value = build(value)
                        if stop(value):
                            return value
                except KeyboardInterrupt:
                    return None
    return None
