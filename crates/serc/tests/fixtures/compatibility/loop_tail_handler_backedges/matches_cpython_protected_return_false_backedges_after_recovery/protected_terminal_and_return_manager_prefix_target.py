def f(self):
    while True:
        try:
            with self.lock:
                count = len(self.buffer)
            try:
                item = self.queue.get()
                if item is not None:
                    with self.lock:
                        self.buffer.append(item)
            except Empty:
                pass
            if self.ready:
                self.flush()
            if self.closed and self.queue.empty():
                return
        except Exception as error:
            report(error)
            if self.closed:
                return
            else:
                pause()
