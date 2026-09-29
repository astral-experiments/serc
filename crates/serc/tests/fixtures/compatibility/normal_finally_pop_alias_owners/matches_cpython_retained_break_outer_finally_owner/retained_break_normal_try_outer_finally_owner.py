def target(queue, manager, work, recover):
    while True:
        item = queue.get()
        if item is None:
            break
        try:
            while True:
                try:
                    with manager() as value:
                        work(item, value)
                    break
                except RuntimeError as error:
                    recover(error)
        except ValueError as error:
            recover(error)
        finally:
            queue.task_done()
