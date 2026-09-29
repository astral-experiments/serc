async def run(value, delayed):
    try:
        while True:
            started = clock()
            try:
                await work(value)
            except Error as error:
                if should_stop(value):
                    return
                report(error)
            finally:
                elapsed = clock() - started
                if elapsed > limit or delayed:
                    if delayed:
                        record(value, context=details())
                    report(value, elapsed)
    except Cancelled:
        raise
    finally:
        cleanup(value)
