async def run(groups, api):
    try:
        for group in groups:
            try:
                for item in group:
                    resource = None
                    try:
                        if api.skip(item):
                            api.record(item)
                            continue
                        resource = api.open(item)
                        if api.failed(resource):
                            api.record(resource)
                            continue
                    finally:
                        if resource:
                            api.close(resource)
                        api.finish_item()
            finally:
                api.finish_group()
    finally:
        api.finish_all()
