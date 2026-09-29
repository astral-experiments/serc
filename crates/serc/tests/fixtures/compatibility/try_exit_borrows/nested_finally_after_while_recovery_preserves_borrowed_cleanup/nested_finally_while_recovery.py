def f(self):
    terminal_error = None
    try:
        self.manager.started()
        self.ready.set()
        while not self.stop.is_set():
            self.manager.heartbeat()
            candidates = self.manager.candidates()
            for key, date in candidates:
                if self.stop.is_set():
                    break
                self.manager.refresh(key, date=date)
            self.wake.wait()
            self.wake.clear()
    except BaseException as error:
        self.error = error
        terminal_error = str(error)
        self.ready.set()
    finally:
        try:
            self.manager.stopped(terminal_error)
        finally:
            self.lock.release()
            self.manager.finished(self)
