def validate(pairs, report):
    for left, right in pairs:
        try:
            if isinstance(left, int):
                assert left == right
            else:
                assert (left == right).all()
        except Exception as error:
            report(error)
            raise
