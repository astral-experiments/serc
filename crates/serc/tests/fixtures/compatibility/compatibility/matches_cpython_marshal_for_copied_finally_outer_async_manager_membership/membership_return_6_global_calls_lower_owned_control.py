async def run(manager, resource, result, selected):
    async with manager:
        try:
            if selected:
                tick()
                tick()
                tick()
                tick()
                tick()
                tick()
                return result
            else:
                return None
        except Error as error:
            report(error)
            return None
        finally:
            if "resource" in locals():
                try:
                    await resource.close()
                except Error:
                    report_failure()
