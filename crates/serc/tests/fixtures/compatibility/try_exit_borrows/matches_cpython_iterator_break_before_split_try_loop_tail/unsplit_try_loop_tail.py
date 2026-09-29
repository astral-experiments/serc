def unsplit_tail(groups, stop):
    timed_out = False
    for values in groups:
        try:
            for value in values:
                if stop(value):
                    timed_out = True
                    break
                consume(value)
            if timed_out:
                break
        except Exception:
            report()
    after()
