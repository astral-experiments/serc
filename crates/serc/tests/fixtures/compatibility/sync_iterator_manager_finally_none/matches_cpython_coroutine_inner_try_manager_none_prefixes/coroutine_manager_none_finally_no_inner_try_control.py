async def target(manager, client):
    try:
        with manager() as session:
            def current_session():
                return session
            session.rollback()
            return False
    finally:
        if client is not None:
            await client.close()
