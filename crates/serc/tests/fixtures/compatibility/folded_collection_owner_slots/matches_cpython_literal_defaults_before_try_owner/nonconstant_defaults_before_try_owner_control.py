async def run(value, invoke):
    if isinstance(value, str):
        value = 0

    async def update(status: str, a=value, b=None, c=None, d=None, e=None, f=None) -> None:
        await invoke(status)

    try:
        if value == 0:
            await update("zero")
    except Exception:
        return
