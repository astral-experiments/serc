def probe(subject, check):
    with manager():
        match subject:
            case [0, _, _]:
                assert check
            case subject.other:
                assert check
            case _:
                assert check
