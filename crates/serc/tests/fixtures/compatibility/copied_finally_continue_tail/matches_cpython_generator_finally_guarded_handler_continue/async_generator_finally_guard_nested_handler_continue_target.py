async def run(receive, remove, recover, retry):
    while True:
        item = await receive()
        success = False
        try:
            yield item
            success = True
        except Exception as error:
            recover(error)
            continue
        finally:
            if success:
                try:
                    await remove(item)
                except ValueError as error:
                    recover(error)
                    if retry(error):
                        continue
                    else:
                        raise
