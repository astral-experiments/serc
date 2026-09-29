async def f(value):
    disabled = value in {"a", "b", "c"}
    staged = False
    try:
        if not disabled:
            staged = await consume(value)
    except Exception:
        staged = False
    return staged
