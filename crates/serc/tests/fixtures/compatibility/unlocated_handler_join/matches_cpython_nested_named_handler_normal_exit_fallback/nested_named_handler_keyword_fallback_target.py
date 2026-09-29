async def nested_handler_fallback(make, work, log, warn, finish):
    try:
        make()
        try:
            await work()
        except Exception as error:
            log(error)
            warn("inner", flush=True)
    except Exception as error:
        log(error)
        warn("outer", flush=True)
    finish()
