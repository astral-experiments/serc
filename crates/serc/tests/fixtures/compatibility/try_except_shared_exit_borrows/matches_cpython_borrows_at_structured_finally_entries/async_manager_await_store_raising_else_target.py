async def fetch(mode, state, first, second, report, manager):
    chosen = state
    result = []
    try:
        if mode == 0:
            with manager():
                result = await first()
        elif mode == 1:
            result = await second()
            chosen = state.updated()
        else:
            raise ValueError(mode)
    except LookupError as error:
        report(error)
        raise
    except Exception as error:
        report(error)
        raise
    finally:
        kind = "none"
        if state.agent is not None:
            if state.is_web():
                kind = "web"
            elif state.is_app():
                kind = "app"
        tags = [chosen.source, kind, "ready" if result else "empty"]
        report(tags)
    return result, chosen
