class Runner:
    async def process(self, items):
        skipped = 0
        for item in items:
            try:
                consume(item)
            except Exception as exc:
                match self.behavior:
                    case Behavior.SKIP:
                        skipped += 1
                        continue
                    case Behavior.RAISE:
                        raise exc
                    case Behavior.RETRY:
                        raise RuntimeError(
                            f'Cannot retry {exc} with behavior {self.behavior}'
                        ) from exc
        return skipped
