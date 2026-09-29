async def target(manager, client):
    try:
        with manager() as session:
            def current_session():
                return session
            try:
                await session.work()
            except ValueError:
                session.rollback()
                return False
            return True
    finally:
        if client is not None:
            await client.close()
