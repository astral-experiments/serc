def f(fetch, use):
    if fetch(
        "key"
    ).value is None:
        use()
