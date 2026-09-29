def f(outer, inner, ignore):
    with outer:
        prepare()
        try:
            with inner as stream:
                value = read(stream)
                first = value.get("first")
                if not isinstance(first, (bytes, bytearray, memoryview)):
                    value.pop("first", None)
                second = value.get("second")
                if not isinstance(second, (bytes, bytearray)):
                    value.pop("second", None)
                return value
        except:
            if ignore:
                return None
            else:
                raise
