async def run(value, invoke):
    if isinstance(value, str):
        value = 0

    async def update(status: str, a=None, b=None, c=None, d=None, e=None) -> None:
        await invoke(status)

    try:
        if value == 0:
            await update("zero")
    except Exception:
        return
