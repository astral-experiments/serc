def f(self):
    while True:
        try:
            try:
                item = self.queue.get()
                if item is not None:
                    self.buffer.append(item)
            except Empty:
                pass
            if self.ready:
                self.flush()
            if self.closed:
                return
        except Exception as error:
            report(error)
            if self.closed:
                return
            else:
                pause()
