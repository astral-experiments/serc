def validate(rows):
    for check in rows:
        assert not (
            all(check) if (hasattr(check, "__iter__") and len(check) > 0) else check
        )
    done()
