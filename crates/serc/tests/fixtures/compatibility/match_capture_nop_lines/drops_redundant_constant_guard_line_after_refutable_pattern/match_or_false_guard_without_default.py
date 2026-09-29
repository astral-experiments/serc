def f(value):
    match value:
        case 1 | 2 if False:
            return 1
