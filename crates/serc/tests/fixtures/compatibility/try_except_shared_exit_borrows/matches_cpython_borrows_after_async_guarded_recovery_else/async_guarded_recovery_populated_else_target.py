async def probe(manager, read, restore, report, opened, changed, restored):
    try:
        async with manager():
            opened = await read()
            if opened:
                changed = await restore()
                if not changed:
                    raise RuntimeError("change failed")
            else:
                restored = await read()
                if not restored:
                    raise RuntimeError("restore failed")
    except Exception as error:
        if changed and not restored:
            try:
                async with manager():
                    restored = await restore()
                report(restored)
            except Exception as recovery_error:
                report(recovery_error)
        status = {"ok": False, "error": str(error), "restored": restored}
    else:
        status = {"ok": True, "restored": restored}
    status["opened"] = opened
    report(status)
    return status
