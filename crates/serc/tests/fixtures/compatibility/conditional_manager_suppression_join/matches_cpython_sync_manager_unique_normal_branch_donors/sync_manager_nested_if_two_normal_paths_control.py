def consume(open_task, stop_when_idle, work, pause):
    while True:
        with open_task() as task:
            if task is None:
                if stop_when_idle:
                    return
            else:
                work(task)
        pause()
