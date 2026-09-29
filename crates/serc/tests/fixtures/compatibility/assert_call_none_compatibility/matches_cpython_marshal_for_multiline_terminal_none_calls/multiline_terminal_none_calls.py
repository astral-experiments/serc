def with_call_same_end(manager, cursor):
    with manager:
        assert cursor.execute(
            "query",
        ).fetchone() is None


def with_call_split_none(manager, cursor):
    with manager:
        assert (
            cursor.execute("query").fetchone()
            is None
        )


def with_call_not_none(manager, cursor):
    with manager:
        assert cursor.execute(
            "query",
        ).fetchone() is not None


def with_call_message(manager, cursor, message):
    with manager:
        assert cursor.execute(
            "query",
        ).fetchone() is None, message()


def with_consecutive(manager, cursor):
    with manager:
        assert cursor.execute(
            "first",
        ).fetchone() is None
        assert cursor.execute(
            "second",
        ).fetchone() is None


def direct_call_same_end(cursor):
    assert cursor.execute(
        "query",
    ).fetchone() is None


def if_call_same_end(manager, cursor):
    with manager:
        if cursor.execute(
            "query",
        ).fetchone() is None:
            action()


def with_name_split_none(manager, value):
    with manager:
        assert (
            value
            is None
        )
