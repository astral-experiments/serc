def conditional_return(flag):
    try:
        if flag:
            return flag
        later = 1
    finally:
        cleanup = 2

async def return_before_handler(work):
    try:
        result = await work()
        return result
    except Exception as error:
        report(error)
        raise
    finally:
        elapsed = timer()
        report(elapsed)
