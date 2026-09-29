def check(outer, inner, enabled, scores):
    with (
        outer,
        inner,
    ):
        if enabled:
            assert scores["value"] < 1.0, f"unexpected value: {scores['value']}"
        else:
            raise ValueError(f"disabled: {enabled}")
