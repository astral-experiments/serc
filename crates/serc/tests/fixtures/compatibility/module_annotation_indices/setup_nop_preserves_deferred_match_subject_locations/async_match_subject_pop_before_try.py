async def probe(attachment, create, log):
    match attachment:
        case SharedPost():
            try:
                return await create(attachment)
            except:
                log()
                raise
        case OtherPost():
            return None
        case _:
            return None
