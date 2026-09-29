def simple(lock, flag):
    with lock:
        try:
            body()
        finally:
            if flag:
                cleanup()

def followed(lock, flag):
    with lock:
        try:
            body()
        finally:
            if flag:
                cleanup()
        after()

def loop(lock, flag, values):
    with lock:
        try:
            for value in values:
                body(value)
        finally:
            if flag:
                cleanup()

async def coroutine(lock, flag):
    with lock:
        try:
            await body()
        finally:
            if flag:
                cleanup()
