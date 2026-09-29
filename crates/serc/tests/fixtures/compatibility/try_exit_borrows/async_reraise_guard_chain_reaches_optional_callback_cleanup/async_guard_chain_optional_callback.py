def make_worker(on_chunk_done):
    async def run(item, suppress):
        completed = False
        try:
            await work(item)
            completed = True
        except Error as error:
            if not recoverable(error):
                raise
            if not suppress:
                raise
            skipped(item)
        finally:
            if completed and on_chunk_done is not None:
                on_chunk_done(1)
    return run
