async def run(queue, work, current_task, emit, finish):
    while True:
        item = await queue.get()
        try:
            await work(item)
        except CancelledError:
            current = current_task()
            if current is not None and current.cancelling():
                emit(current)
                raise
        except Exception as error:
            emit(error)
        finally:
            await finish(item)
        emit(item)
