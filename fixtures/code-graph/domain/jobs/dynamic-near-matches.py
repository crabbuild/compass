class Fake:
    def task(self, name):
        return lambda function: function

fake = Fake()

@fake.task(name=get_dynamic_name())
def not_a_celery_task():
    pass
