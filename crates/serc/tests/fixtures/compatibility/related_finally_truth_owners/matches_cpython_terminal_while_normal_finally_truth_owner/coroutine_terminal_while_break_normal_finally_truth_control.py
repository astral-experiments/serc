async def f(manager, chunks, pending, work, cleanup):
    try:
        with manager:
            for chunk in chunks:
                try:
                    if not pending:
                        continue
                    while pending:
                        await work()
                        break
                finally:
                    if chunk:
                        await cleanup(chunk)
    finally:
        cleanup(None)
