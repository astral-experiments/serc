def f(factory, config, items):
    task = factory([item for item in items], config, items)
    try:
        namespace = config.namespace if config.enabled else None
        return task.run(namespace)
    except Exception:
        return None
