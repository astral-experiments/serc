def f(items, ready, report, recover, after, cleanup):
    try:
        try:
            for item in items:
                if ready(item):
                    break
            else:
                report()
        except Error:
            recover()
        after()
    finally:
        cleanup()
