async def collect(values, output):
    async for value in values:
        match value:
            case Included():
                await output.put(value)
            case Skipped():
                pass
