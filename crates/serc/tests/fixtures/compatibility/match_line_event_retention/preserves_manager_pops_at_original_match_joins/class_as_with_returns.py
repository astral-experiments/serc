def f(subject, manager, First, Second, make):
    with manager:
        match subject:
            case First() as first:
                return make(first)
            case Second() as second:
                return make(second)
