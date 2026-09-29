def f(obj, active, checked):
    if obj.ready:
        assert obj.ready
    prepared = obj.prepare()
    if obj.active:
        if obj.check():
            assert obj.valid
        else:
            assert obj.other
