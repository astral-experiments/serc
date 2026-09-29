def run(work, probe, report, make, value):
    try:
        work()
        try:
            report(value)
            try:
                state = probe()
                if state:
                    work()
                else:
                    pass
            except Exception as inner:
                pass
        except Exception as outer:
            report(outer)
            try:
                work()
            except:
                pass
            report(value)
        result = make(value)
        report(result)
        return result
    except Exception:
        return None
