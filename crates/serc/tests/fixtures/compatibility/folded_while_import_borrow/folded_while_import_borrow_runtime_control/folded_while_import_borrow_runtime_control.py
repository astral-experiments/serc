class Worker:
    def run(self, seed):
        attempt = 0
        while ready():
            attempt += 1
            token = f'{self.context}:{seed}:{attempt}'
            from support import deadline

            @deadline(10)
            def produce(token=token):
                return make_value(token, self.config)

            try:
                value = produce()
                if value is None:
                    continue
            except (TimeoutError, SystemError):
                continue
            return value
