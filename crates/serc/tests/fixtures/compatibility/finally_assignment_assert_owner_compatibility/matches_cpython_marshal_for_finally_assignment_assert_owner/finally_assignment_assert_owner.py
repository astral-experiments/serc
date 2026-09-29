class Tracker:
    def finish_batch(self):
        try:
            assert self.batch is not None
            if self.batch:
                widget = self.receiver()
                assert widget is not None
                widget.generate("changed", data=self.batch)
        finally:
            self.batch = None
