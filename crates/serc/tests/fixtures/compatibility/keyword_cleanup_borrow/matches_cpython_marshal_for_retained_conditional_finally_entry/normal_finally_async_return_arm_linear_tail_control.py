async def f(self, ready, manager, report):
    status = "success"
    count = 0
    try:
        if ready:
            async with manager as response:
                result = await response.read()
                count = len(result)
                return result
        report(ready)
    except Exception as error:
        report(error)
        status = "error"
    finally:
        elapsed = self.clock() - self.started
        tags = (status, count, self.tags)
        report(elapsed, tags)
    return None
