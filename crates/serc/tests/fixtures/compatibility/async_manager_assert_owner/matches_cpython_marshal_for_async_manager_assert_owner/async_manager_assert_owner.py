async def reconcile(dry_run: bool = True) -> None:
    """Reconcile recorded items with their current state."""

    async def remove(item: Record) -> None:
        if not dry_run:
            item.status = State.REMOVED
            await database().upsert(item)

    items = await database().get_all()
    for item in items:
        with item_context(item=item):
            if item.kind == Kind.PRIMARY:
                if item.status == State.READY:
                    try:
                        async with Client() as client:
                            assert item.info.external_id
                            response = await client.get(item.info.external_id)
                            resources = response.get("resources", ())
                            if not resources:
                                logger.info(f"Item {item.id} has no resources")
                                await remove(item)
                                continue
                    except Exception as error:
                        logger.exception(f"Unknown status of {item.id}: {error}")
            elif item.kind.is_secondary():
                try:
                    await keepalive(
                        id=item.info.external_id,
                        session_data=str(item.metadata),
                    )
                except Exception as error:
                    logger.info(f"Item {item.id} has no active session: {error}")
                    await remove(item)
