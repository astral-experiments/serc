def f(obj, active, checked):
    prepared = obj.prepare()
    if obj.active:
        if obj.check():
            assert obj.valid
        else:
            assert obj.other
    else:
        assert obj.active
