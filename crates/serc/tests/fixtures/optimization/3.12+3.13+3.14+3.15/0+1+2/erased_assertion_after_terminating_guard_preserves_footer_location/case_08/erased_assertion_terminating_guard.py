def f(value, items, obj, flag):
    if value is None:
        return value
    else:
        if items is not None:
            match flag:
                case 0:
                    obj.work()
                case _:
                    obj.other()
            raise ValueError(value)
        assert False
