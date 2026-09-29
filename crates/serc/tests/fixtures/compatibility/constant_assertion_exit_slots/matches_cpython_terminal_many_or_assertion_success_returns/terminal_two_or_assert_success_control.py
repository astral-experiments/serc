def check(active, value, values):
    if active:
        assert value == 'max' or (str(value) == '4' and any(item for item in values)), 'unsupported'
