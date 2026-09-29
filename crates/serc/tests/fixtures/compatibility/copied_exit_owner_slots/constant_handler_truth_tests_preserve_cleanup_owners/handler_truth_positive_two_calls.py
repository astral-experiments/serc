async def run(flag):
    try:
        await work()
    except Exception:
        report0()
        report1()
        return None
    finally:
        if flag:
            cleanup()
        finish()
