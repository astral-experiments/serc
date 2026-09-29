def f(stages, flag, duration, clock, work, release):
    for index, stage in enumerate(stages):
        started = clock()
        phase = f"stage {index}: preparing"
        nodes: list[str] = []
        try:
            try:
                phase = "draining"
                nodes = work(stage, duration)
                work(stage)
            finally:
                if flag and duration:
                    release(nodes)
        except Exception as error:
            raise Error(phase, error) from error
        report(clock() - started)
