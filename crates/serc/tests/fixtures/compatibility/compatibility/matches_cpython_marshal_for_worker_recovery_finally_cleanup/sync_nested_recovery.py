def worker(queue, output, metric, logger, sentinel):
    while True:
        item = queue.get()
        try:
            if item is sentinel:
                queue.put(sentinel)
                return
            try:
                prepare(item)
                result = item.convert()
                output.put(result)
            except Error:
                logger.report(item)
                try:
                    metric.update(item)
                except Error:
                    logger.report("failed")
        finally:
            queue.task_done()
