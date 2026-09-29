async def f(self):
    try:
        try:
            self.address = resolver.resolve(self.original)
            if not self.local:
                self.peer = self.address
        except Exception:
            pass
        await pause()
    except Exception:
        recover()
