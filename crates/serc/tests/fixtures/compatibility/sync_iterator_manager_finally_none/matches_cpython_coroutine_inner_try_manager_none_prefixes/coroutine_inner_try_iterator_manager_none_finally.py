async def target(manager, items, client):
    try:
        with manager() as session:
            def current_session():
                return session
            try:
                for item in items:
                    if item:
                        session.rollback()
                        return False
            except Exception:
                raise
            return True
    finally:
        if client is not None:
            await client.close()
