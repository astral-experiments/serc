async def f(self, choice, stop, manager, work, report):
    try:
        if choice:
            if stop:
                return
            with manager:
                await work()
        else:
            with manager:
                await work()
    except Exception as error:
        report(error)
    finally:
        self.pending -= 1
        report(self.pending, self.tags)
    report("done")
