def calls(target, obj, values, mapping):
    target()
    target(1, 2, 3)
    target(1, option=2)
    target(*values)
    target(**mapping)
    target(1, *values, option=2, **mapping)
    obj.method()
    obj.child.method(1, 2)
    obj.attribute
    obj.attribute = values[0]
    obj[0]
    obj[1:10:2]
    obj[0] = values[-1]
    del obj.attribute
    del obj[0]
