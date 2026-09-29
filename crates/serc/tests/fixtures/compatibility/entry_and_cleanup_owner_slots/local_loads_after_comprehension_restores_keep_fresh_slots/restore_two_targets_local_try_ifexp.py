def f(factory, config, items):
    task = factory(values=[left + right for left, right in items], juice=config.juice)
    try:
        namespace = config.namespace if config.enabled else None
        return task.run(namespace)
    except Exception:
        return None
