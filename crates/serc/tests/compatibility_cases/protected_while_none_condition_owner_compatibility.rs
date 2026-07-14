use crate::common;

#[test]
fn matches_cpython_marshal_for_protected_while_none_condition_owners() {
    let source = r#"
def prefect_style(manager, selected_row, use, advance):
    with manager:
        while selected_row is None:
            use(selected_row)
            selected_row = advance(selected_row)


def pwndbg_style(manager, frame, use):
    with manager:
        while frame is not None:
            use(frame)
            frame = frame.parent()


def psycopg_sync_style(manager, cursor):
    with manager:
        while cursor.read_row() is not None:
            pass


def named_expression_style(manager, iterator, next_func, result):
    with manager:
        while (item := next_func(iterator)) is not None:
            result.append(item)


def one_break_style(manager, value, advance, use):
    with manager:
        while value is not None:
            if value.finished:
                break
            use(value)
            value = advance(value)


def nested_with_style(first, second, frame, use):
    with first:
        with second:
            while frame is not None:
                use(frame)
                frame = frame.parent()


def outer_try_style(manager, frame, use):
    try:
        with manager:
            while frame is not None:
                use(frame)
                frame = frame.parent()
    except ValueError:
        use(None)


async def coroutine_style(manager, frame, use, pause):
    await pause()
    with manager:
        while frame is not None:
            use(frame)
            frame = frame.parent()


async def psycopg_async_style(manager, cursor):
    async with manager:
        while (await cursor.fetchone()) is not None:
            pass


def cursor_style(conn_cls, dsn, fmt_out, row_factory, faker, fetch):
    with conn_cls.connect(dsn) as conn, conn.transaction(force_rollback=True):
        with conn.cursor(binary=fmt_out, row_factory=row_factory) as cur:
            cur.execute(faker.drop_stmt)
            cur.execute(faker.create_stmt)
            with faker.find_insert_problem(conn):
                cur.executemany(faker.insert_stmt, faker.records)
            cur.execute(faker.select_stmt)

            if fetch == "one":
                while cur.fetchone() is not None:
                    pass
            elif fetch == "many":
                while cur.fetchmany(3):
                    pass
            elif fetch == "all":
                cur.fetchall()
            elif fetch == "iter":
                for rec in cur:
                    pass


def cursor_raw_style(conn_cls, dsn, fmt_out, row_factory, faker, fetch, ph):
    with conn_cls.connect(dsn) as conn:
        with conn.transaction(force_rollback=True):
            with conn.cursor(binary=fmt_out, row_factory=row_factory) as cur:
                cur.execute(faker.drop_stmt)
                cur.execute(faker.create_stmt)
                with faker.find_insert_problem(conn):
                    cur.executemany(faker.insert_stmt, faker.records)
                cur.execute(ph(cur, faker.select_stmt))

                if fetch == "one":
                    while cur.fetchone() is not None:
                        pass
                elif fetch == "many":
                    while cur.fetchmany(3):
                        pass
                elif fetch == "all":
                    cur.fetchall()
                elif fetch == "iter":
                    for rec in cur:
                        pass


def copy_style(conn_cls, dsn, fmt, faker, method, set_types, sql):
    with conn_cls.connect(dsn) as conn:
        with conn.cursor(binary=fmt) as cur:
            cur.execute(faker.drop_stmt)
            cur.execute(faker.create_stmt)
            with faker.find_insert_problem(conn):
                cur.executemany(faker.insert_stmt, faker.records)

            stmt = sql.SQL("copy ({})").format(faker.table_name)
            with cur.copy(stmt) as copy:
                if set_types:
                    copy.set_types(faker.types_names)
                if method == "read":
                    while copy.read():
                        pass
                elif method == "iter":
                    list(copy)
                elif method == "row":
                    while copy.read_row() is not None:
                        pass
                elif method == "rows":
                    list(copy.rows())


def truthy_name_control(manager, values, use):
    with manager:
        while values:
            use(values.pop())


def truthy_call_control(manager, cursor, use):
    with manager:
        while cursor.read():
            use(cursor)


def ownerless_name_control(frame, use):
    while frame is not None:
        use(frame)
        frame = frame.parent()


def terminal_break_following_control(manager, frame, use):
    with manager:
        while frame is not None:
            break
    use(frame)


def terminal_return_following_control(manager, frame, use):
    with manager:
        while frame is not None:
            return
    use(frame)


def terminal_raise_following_control(manager, frame, use):
    with manager:
        while frame is not None:
            raise ValueError
    use(frame)


def intervening_for_control(manager, workers, use):
    with manager:
        for worker in workers:
            while worker.ident is None:
                use(worker)
"#;

    common::assert_matches_cpython_marshal(
        source,
        "protected_while_none_condition_owner_compatibility.py",
    );
}
