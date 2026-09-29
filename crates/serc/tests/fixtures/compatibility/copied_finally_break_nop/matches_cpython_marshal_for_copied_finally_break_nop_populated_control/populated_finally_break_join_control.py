async def start(context):
    async def worker():
        while True:
            item = await context.queue.get()
            if item is None:
                context.queue.task_done()
                break
            backoff = 1
            retry_count = 0
            while True:
                token = None
                try:
                    token = pin(item)
                    async with session():
                        await process(item)
                    record(item)
                    break
                except (RetryError, StopError) as error:
                    report(error)
                    await context.results.put(error)
                    break
                except Exception as error:
                    report(error)
                    await sleep(backoff)
                    backoff = min(backoff * 2, 10)
                    retry_count += 1
                finally:
                    if token is not None:
                        reset(token)
                    cleanup_finished()
            context.queue.task_done()
    context.workers = [create_task(worker()) for _ in range(context.parallelism)]
