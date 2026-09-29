class Processor:
    def execute(self, key, text, mode, payload):
        worker: object | None = None
        try:
            workspace = self.prepare(key)
            self.journal.record(key, phase="starting")
            session, environment = self.backend.create(mode, key)
            self.journal.record(key, session=session, environment=environment)
            worker = self.backend.start(session, environment, workspace)
            self.journal.record(key, phase="running", identifier=self.backend.identifier(worker))
            self.backend.submit(session, self.render(text, payload))
            terminal = False
            for event in self.backend.events(session, worker):
                self.journal.append(key, event)
                projection = project(event)
                if projection.terminal:
                    terminal = True
                    if not self.action_required(key):
                        self.journal.record(key, phase=projection.phase or "complete", error=projection.error)
                    break
            if not terminal:
                raise RuntimeError("stream ended before completion")
        except ActionRequired as error:
            self.journal.record(key, phase="needs input", error=None, required_action=error.action)
        except Exception as error:
            if not self.action_required(key):
                self.journal.record(key, phase="failed", error=str(error))
        finally:
            if worker is not None:
                try:
                    self.backend.stop(worker)
                except Exception as error:
                    if not self.action_required(key):
                        self.journal.record(key, phase="failed", error=f"cleanup failed: {error}")
