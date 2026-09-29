async def validate(mode, selected, left, right):
    if mode == 0:
        result = left
        assert left
        pass
    elif mode == 1:
        result = right
        if selected:
            assert len(left) == len(right)
        else:
            assert len(left) == 0
        pass
    else:
        raise ValueError(mode)
    assert result
