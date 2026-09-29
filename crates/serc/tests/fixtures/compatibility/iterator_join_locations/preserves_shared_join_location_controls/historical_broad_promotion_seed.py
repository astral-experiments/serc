def loop_join(items, enabled, alternate, ready, begin):
    for item in items:
        if enabled:
            if ready(item):
                begin()
        elif alternate:
            if ready(item):
                break
