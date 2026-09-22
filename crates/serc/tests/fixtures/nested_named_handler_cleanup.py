def f(work, manager, choose, enabled, gather, log):
    try:
        work()
    except OSError as error:
        if choose(error):
            if enabled():
                items = []
                with manager:
                    items = [item.name for item in gather()]
                log(items)
        else:
            log(error)
    else:
        log(None)
