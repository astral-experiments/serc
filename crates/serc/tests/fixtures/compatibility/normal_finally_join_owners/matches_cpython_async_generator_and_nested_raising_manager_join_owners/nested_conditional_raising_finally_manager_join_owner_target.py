def run(first, second, third, work, close, collect, report, state):
    with first(), second(), third():
        try:
            work()
        finally:
            failures = collect()
            try:
                close()
            except Exception as error:
                failures.append(error)
            if failures:
                if state[0] is not None:
                    report(errors=[repr(error) for error in failures])
                else:
                    raise RuntimeError(failures)
