def f(value, request):
    if value.limit is not None:
        assert request.paged, "expected paged"
        assert value.limit == request.limit, "different limit"
    else:
        assert not request.paged or request.limit is None, "missing limit"
