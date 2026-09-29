async def target(resource, retry):
    try:
        print("starting")
        result = await resource.run()
        print(result)
        print("ready")
    except Exception as error:
        print("failed", error)
        try:
            from math import isfinite as should_retry
        except ImportError:
            should_retry = bool
        if should_retry(retry):
            try:
                print("recovered", resource.recover())
            except Exception:
                print("unavailable")
    finally:
        print("closing")
        await resource.close()
    print("done")
