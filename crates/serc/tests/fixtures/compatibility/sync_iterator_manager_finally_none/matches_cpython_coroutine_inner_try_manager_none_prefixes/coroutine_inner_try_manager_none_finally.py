async def target(manager, condition, client):
    try:
        with manager() as session:
            def current_session():
                return session
            try:
                if condition:
                    session.rollback()
                    return False
            except Exception:
                raise
            return True
    finally:
        if client is not None:
            await client.close()
