async def f(source, enabled):
    try:
        value = await source.prepare()
        try:
            await source.reset(value)
        except Exception:
            pass
        disabled = source.setting("DISABLE", "0").lower() in {"1", "true", "yes", "on"}
        staged = False
        try:
            if not disabled:
                account = source.setting("ACCOUNT") or "default"
                staged = await source.stage(value, account)
        except Exception as error:
            source.record(error)
            staged = False
        return staged
    except Exception:
        return False
