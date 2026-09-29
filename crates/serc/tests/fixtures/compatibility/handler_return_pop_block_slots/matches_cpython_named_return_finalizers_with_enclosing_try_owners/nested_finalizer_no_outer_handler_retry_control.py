async def poll(worker, retry):
    await worker.ready.wait()
    while True:
        finish = await worker.reserve()
        if finish is None:
            return
        try:
            if worker.abandoned:
                await worker.cancel(finish=finish)
                finish = None
                return
            result = await worker.retrieve(finish=finish)
            if result.pending:
                continue
            worker.commit(result, finish)
            finish = None
            return
        except CancelledError:
            raise
        except Exception as error:
            if retry(error):
                continue
            await worker.cancel(error=error, finish=finish)
            finish = None
            return
        finally:
            if finish is not None:
                finish.release()
