async def retry(worker, manager=None):
    count = 0
    while count < 3:
        try:
            result = await worker()
            return result.value
        except Exception as error:
            report(error)
            count += 1
            if manager is not None:
                manager.update({"failed": 1})
    if manager is not None:
        manager.update({"dropped": 1})
    return None
