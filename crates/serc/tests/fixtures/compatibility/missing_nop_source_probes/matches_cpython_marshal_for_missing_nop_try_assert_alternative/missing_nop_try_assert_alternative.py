def choose(self):
    try:
        if not self.preferred.empty():
            item = self.preferred.get_nowait()
            assert item.metadata.private is not None
        else:
            item = self.fallback.get_nowait()
    except EmptyQueue:
        return None
    return item
