async def terminal_try_or(left, right):
    try:
        if left is not None:
            return await action(left)
        elif right is not None:
            value = await action(right)
            check(value)
            return value.item()
        assert left or right, "missing value"
    except ValueError as error:
        raise RuntimeError("bad value") from error
    except Exception as error:
        observe(error)
        raise RuntimeError("bad value") from error
