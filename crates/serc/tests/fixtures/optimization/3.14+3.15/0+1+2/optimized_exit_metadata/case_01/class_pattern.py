def probe(subject):
    match subject:
        case Record(value=0):
            assert False
        case constants.other:
            assert True
        case _:
            assert False
