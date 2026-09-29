def conditional_handler_before_finally(condition):
    try:
        work()
    except Error as error:
        report(error)
        if condition:
            action()
    finally:
        cleanup()

async def conditional_async_handler_before_finally(condition):
    try:
        await work()
    except Error as error:
        report(error)
        if condition:
            await action()
    finally:
        cleanup()

def conditional_with_in_handler(condition, manager):
    try:
        work()
    except Error as error:
        with manager:
            if condition:
                action()
    finish()
