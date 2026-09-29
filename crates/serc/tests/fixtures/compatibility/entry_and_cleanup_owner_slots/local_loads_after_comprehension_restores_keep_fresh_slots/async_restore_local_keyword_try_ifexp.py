async def f(factory, config, items):
    task = factory(values=[item for item in items], juice=config.juice)
    try:
        namespace = config.namespace if config.enabled else None
        async with task.use(namespace):
            return await task.run()
    except Exception:
        return None
