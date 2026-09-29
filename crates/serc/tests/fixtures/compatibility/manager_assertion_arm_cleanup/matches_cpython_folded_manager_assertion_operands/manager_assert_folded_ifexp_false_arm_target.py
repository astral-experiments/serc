def verify(manager, value, expected, choose):
    with manager():
        assert (
            value == expected
            if choose
            else ""
        )
