async def process(self, work, active, response):
    while active:
        try:
            try:
                response = await work()
            except ConnectionError:
                break
            if response is None:
                break
            self.consume(response)
        finally:
            self.task = None
            if self.transport is None and response is not None:
                self.report(response)
        self.advance()
    self.finish()
    return response
