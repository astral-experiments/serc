def target():
    bindings = (1, 2, 3, 4, 5, 6, 7, 8, 9)

    def configure(connection):
        connection.apply(bindings)

    engine = make_engine(configure=configure)
    sessions = make_sessions(engine)
    try:
        result, first_id, second_id = make_result(sessions)
        assert result.attempt_id is not None
        with sessions() as session:
            plan = make_plan(session, result.attempt_id)
        assert plan.ready is True
    finally:
        engine.close()
